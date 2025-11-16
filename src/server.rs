use std::{collections::VecDeque, path::Path, sync::Arc, time::Duration};

use actix_web::{
    App, HttpResponse, HttpResponseBuilder, HttpServer, ResponseError,
    dev::{HttpServiceFactory, Url},
    error::PayloadError,
    get,
    http::{StatusCode, Uri, uri},
    post,
    web::{self, Data},
};
use clap::error;
use once_cell::sync::Lazy;
use paperless_api_client::{
    Client,
    types::{CustomField, Document, Tag},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    join, spawn,
    sync::{Mutex, RwLock},
    task::{JoinError, spawn_blocking},
};

static DOCID_REGEX: Lazy<Regex> = regex_static::lazy_regex!(r"documents/(\d*)");

static PROCESSING_QUEUE: Lazy<tokio::sync::RwLock<VecDeque<DocumentProcessingRequest>>> =
    Lazy::new(|| RwLock::new(VecDeque::new()));

// shutdown bit, when this is set to true the document processing pipeline will be shut down
static STOP_FLAG: Lazy<tokio::sync::RwLock<bool>> = Lazy::new(|| RwLock::new(false));

// model will only be initialized and stored if there are documents that need processing
static MODEL_SINGLETON: Lazy<tokio::sync::Mutex<Option<LLModelExtractor>>> =
    Lazy::new(|| Mutex::new(None));

use crate::{
    config::{self, Config},
    extract::{LLModelExtractor, ModelError},
    requests,
    types::{custom_field_learning_supported, FieldError},
};

#[non_exhaustive]
#[derive(Debug, PartialEq)]
enum ProcessingType {
    CustomFieldPrediction,
    CorrespondentSuggest,
}

/// Most processing of document will involve feeding document data throuh a large languae model.
/// Since LLM are notoriously resource intensive a task queue is used in order to facilitate asyncronous
/// batched processinsg a container is required to hold the queue of processing requests
struct DocumentProcessingRequest {
    document: Document,
    processing_type: ProcessingType,
}

#[derive(Debug, Error)]
enum DocumentProcessingError {
    #[error("Could not start LLM processinsg thread: {0}")]
    SpawningProcessingThreadFailed(#[from] JoinError),
    #[error(transparent)]
    ModelProcessingError(#[from] ModelError),
    #[error(transparent)]
    PaperlessCommunicationError(#[from] paperless_api_client::types::error::Error),
    #[error(transparent)]
    ExtractionError(#[from] FieldError)
}

async fn handle_correspondend_suggest(
    doc: &Document,
    api_client: &mut Client,
) -> Result<(), DocumentProcessingError> {
    let crrspndts = requests::fetch_all_correspondents(api_client).await;
    let crrspndts_suggest_schema = crate::types::schema_from_correspondents(crrspndts.as_slice());

    let doc_data = serde_json::to_value(&doc.content).unwrap();

    let extracted_correspondent = spawn_blocking(move || {
        let mut model_singleton = MODEL_SINGLETON.blocking_lock();
        if let Some(model) = model_singleton.as_mut() {
            model.extract(&doc_data, &crrspndts_suggest_schema, false)
        } else {
            Err(crate::extract::ModelError::ModelNotLoaded)
        }
    })
    .await??;

    let new_crrspndt = extracted_correspondent.to_correspondent(&crrspndts)?;
    requests::update_doc_correspondent(api_client, doc, &new_crrspndt).await?;

    Ok(())
}

async fn handle_custom_field_prediction(
    doc: &mut Document,
    api_client: &mut Client,
) -> Result<(), DocumentProcessingError> {
    // fetch all custom field definitions for fields on the document that need to be filled
    let relevant_custom_fields: Vec<CustomField> = requests::get_custom_fields_by_id(
        api_client,
        doc.custom_fields.as_ref().map(|cfis| {
            cfis.iter()
                .filter(|cfi| cfi.value.is_none())
                .map(|cfi| cfi.field)
                .collect()
        }),
    )
    .await
    // this filters out all custom fields that are currently unsupported
    .into_iter().filter(|cf| {
        let learning_supported = custom_field_learning_supported(cf);
        if !learning_supported {
            log::warn!("Custom Fields with name `{}` are using an unsupported custom field type, will be ignored!", cf.name);
        }
        learning_supported
    }).collect();

    let mut updated_custom_fields = false;

    for cf in relevant_custom_fields {
        let doc_data = serde_json::to_value(&doc).unwrap();

        if let Some(field_grammar) = crate::types::schema_from_custom_field(&cf) {
            let extracted_cf = spawn_blocking(move || {
                let mut model_singleton = MODEL_SINGLETON.blocking_lock();
                if let Some(model) = model_singleton.as_mut() {
                    model.extract(&doc_data, &field_grammar, false)
                } else {
                    Err(crate::extract::ModelError::ModelNotLoaded)
                }
            })
            .await??;

            if let Ok(cf_value) = extracted_cf.to_custom_field_instance(&cf).map_err(|err| {
                log::error!("{err}");
                err
            }) {
                // update document custom fields on server side
                // sending the updated document to the server will happen afterwards
                log::debug!("Extracted custom field for document {}\n {:#?}", doc.id, cf_value);
                doc.custom_fields.as_mut().map(|doc_custom_fields| {
                    for doc_cf_i in doc_custom_fields.iter_mut() {
                        if doc_cf_i.field == cf_value.field {
                            *doc_cf_i = cf_value.clone()
                        }
                    }
                });
                updated_custom_fields = true;
            }
        }
    }

    // after custom fields have been changed on the document sync state back to paperless
    if updated_custom_fields {
        let mut cf_list = Vec::new();
        if let Some(doc_fields) = doc.custom_fields.as_ref() {
            for cf in doc_fields {
                cf_list.push(cf.clone());
            }
        }
        requests::update_document_custom_fields(api_client, doc, &cf_list).await?;
    }

    Ok(())
}

#[post("/fill/custom_fields")]
async fn custom_field_prediction(
    params: web::Json<WebhookParams>,
    status_tags: Data<PaperlessStatusTags>,
    api_client: Data<Client>,
    config: Data<Config>,
    document_pipeline: web::Data<tokio::sync::mpsc::UnboundedSender<DocumentProcessingRequest>>,
) -> Result<HttpResponse, WebhookError> {
    let _ = params
        .handle_request(
            status_tags,
            api_client,
            config,
            document_pipeline,
            ProcessingType::CustomFieldPrediction,
        )
        .await?;
    Ok(HttpResponse::Accepted().into())
}

#[derive(Debug, thiserror::Error)]
enum WebhookError {
    #[error("Document with id `{0}` does not exist!")]
    DocumentDoesNotExist(i64),
    #[error("Document ID is not a valid integer!")]
    InvalidDocumentId,
    #[error("Could not parse Document ID from `document_url` field!")]
    DocumentUrlParsingIDFailed,
    #[error("Document Url points to a server unrelated to this configuration. Ignoring Request")]
    ReceivedRequestFromUnconfiguredServer,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebhookParams {
    document_url: String,
}

impl ResponseError for WebhookError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        match self {
            WebhookError::ReceivedRequestFromUnconfiguredServer => StatusCode::UNAUTHORIZED,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl WebhookParams {
    async fn handle_request(
        &self,
        status_tags: Data<PaperlessStatusTags>,
        api_client: Data<Client>,
        config: Data<Config>,
        document_pipeline: web::Data<tokio::sync::mpsc::UnboundedSender<DocumentProcessingRequest>>,
        req_type: ProcessingType,
    ) -> Result<(), WebhookError> {
        let doc_url: Uri = self.document_url.parse().unwrap();
        let configured_paperless_server: Uri = config.paperless_server.parse().unwrap();
        // verify document host and configured paperless instance are the same host to avoid handling requests from other paperless instances
        if doc_url.host().is_some_and(|rs| {
            configured_paperless_server
                .host()
                .is_some_and(|cs| cs == rs)
        }) {
            if let Some(doc_id_cap) = DOCID_REGEX.captures(doc_url.path()) {
                if let Some(doc_id) = doc_id_cap
                    .get(1)
                    .and_then(|v| v.as_str().parse::<i64>().ok())
                {
                    let mut api_client =
                        Arc::<Client>::make_mut(&mut api_client.into_inner()).clone();
                    if let Ok(mut doc) = api_client.documents().retrieve(doc_id, None, None).await {
                        // if documents has no processing tag set it
                        if !doc.tags.contains(&status_tags.processing.id) {
                            let mut updated_doc_tags = doc.tags.clone();
                            updated_doc_tags.push(status_tags.processing.id);
                            let _ = requests::update_document_tag_ids(
                                &mut api_client,
                                &mut doc,
                                &updated_doc_tags,
                            )
                            .await;
                        }
                        let _ = document_pipeline.send(DocumentProcessingRequest {
                            document: doc,
                            processing_type: req_type,
                        });
                        Ok(())
                    } else {
                        Err(WebhookError::DocumentDoesNotExist(doc_id))
                    }
                } else {
                    Err(WebhookError::InvalidDocumentId)
                }
            } else {
                Err(WebhookError::DocumentUrlParsingIDFailed)
            }
        } else {
            Err(WebhookError::ReceivedRequestFromUnconfiguredServer)
        }
    }
}

#[post("/suggest/correspondent")]
async fn suggest_correspondent(
    params: web::Json<WebhookParams>,
    status_tags: Data<PaperlessStatusTags>,
    api_client: Data<Client>,
    config: Data<Config>,
    document_pipeline: web::Data<tokio::sync::mpsc::UnboundedSender<DocumentProcessingRequest>>,
) -> Result<HttpResponse, WebhookError> {
    let _ = params
        .handle_request(
            status_tags,
            api_client,
            config,
            document_pipeline,
            ProcessingType::CorrespondentSuggest,
        )
        .await?;
    Ok(HttpResponse::Accepted().into())
}

struct DocumentProcessingApi;

impl HttpServiceFactory for DocumentProcessingApi {
    fn register(self, config: &mut actix_web::dev::AppService) {
        custom_field_prediction.register(config);
        suggest_correspondent.register(config);
    }
}

// future performance optimization needs to focus on this function, it should dispatch to batch processing of documents
// or could combine requests to the same document in the queue.
async fn document_processor(
    config: Config,
    mut api_client: Client,
    status_tags: PaperlessStatusTags,
) {
    while !*STOP_FLAG.read().await {
        let processing_queue_size = PROCESSING_QUEUE.read().await.len();
        if processing_queue_size > 0 {
            let model_path = config.model.clone();
            {
                let mut model_singleton = MODEL_SINGLETON.lock().await;
                *model_singleton = spawn_blocking(move || {
                    LLModelExtractor::new(&Path::new(&model_path), config.num_gpu_layers, None)
                })
                .await
                .map_err(|err| {
                    log::error!("Error loading Model! {err}");
                    err
                })
                .ok();
            }
            let mut doc_process_req = PROCESSING_QUEUE
                .write()
                .await
                .pop_front()
                .expect("Size is greater 0 so there must be a document in the queue");
            if doc_process_req.processing_type == ProcessingType::CorrespondentSuggest {
                let _ =
                    handle_correspondend_suggest(&doc_process_req.document, &mut api_client).await;
            } else if doc_process_req.processing_type == ProcessingType::CustomFieldPrediction {
                let _ =
                    handle_custom_field_prediction(&mut doc_process_req.document, &mut api_client)
                        .await;
            } else {
                log::warn!(
                    "Unimplemented processing type {:?}! Ignoring request …",
                    doc_process_req.processing_type
                )
            }
        } else {
            // No Documents need processing drop model
            let mut model_singleton = MODEL_SINGLETON.lock().await;
            let _ = model_singleton.take();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// this function is just here to receive documents for processing from the different api endpoints
/// all documents received via the channel are put into a linked list of documents that need processing
/// the reason for this is that this way the document processor can inspect the state of the document queue and
/// make smart decisions on how to process the documents for maximum efficiency
async fn document_request_funnel(
    mut processing_request_channel: tokio::sync::mpsc::UnboundedReceiver<DocumentProcessingRequest>,
) {
    while let Some(prc_req) = processing_request_channel.recv().await {
        log::debug!("Received Request for Document {:#?}", prc_req.document.id);
        let mut doc_queue = PROCESSING_QUEUE.write().await;
        doc_queue.push_back(prc_req);
    }

    *STOP_FLAG.write().await = true;
}

#[derive(Debug, Clone)]
struct PaperlessStatusTags {
    processing: Tag,
    finished: Tag,
}

pub async fn run_server(
    config: Config,
    processing_tag: Tag,
    finished_tag: Tag,
    paperless_api_client: Client,
) -> Result<(), std::io::Error> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<DocumentProcessingRequest>();

    let status_tags = PaperlessStatusTags {
        processing: processing_tag,
        finished: finished_tag,
    };

    let doc_processor = spawn(document_processor(
        config.clone(),
        paperless_api_client.clone(),
        status_tags.clone(),
    ));
    let doc_to_process_queue = spawn(document_request_funnel(rx));
    let webhook_server = HttpServer::new(move || {
        let app = App::new()
            .app_data(Data::new(tx.clone()))
            .app_data(Data::new(config.clone()))
            .app_data(Data::new(paperless_api_client.clone()))
            .app_data(Data::new(status_tags.clone()))
            .service(DocumentProcessingApi);
        app
    })
    .bind(("0.0.0.0", 8123))?
    .run();

    let _ = join!(webhook_server, doc_to_process_queue, doc_processor);

    Ok(())
}
