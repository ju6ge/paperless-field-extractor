use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::Arc,
    time::Duration,
};

use actix_web::{
    App, HttpResponse, HttpServer, ResponseError,
    dev::HttpServiceFactory,
    http::{StatusCode, Uri},
    post,
    web::{self, Data},
};
use itertools::Itertools;
use once_cell::sync::Lazy;
use paperless_api_client::{
    Client,
    types::{CustomField, CustomFieldInstance, Document, Tag},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    join, spawn,
    sync::{Mutex, RwLock},
    task::{JoinError, spawn_blocking},
};
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

static DOCID_REGEX: Lazy<Regex> = regex_static::lazy_regex!(r"documents/(\d*)");

static PROCESSING_QUEUE: Lazy<tokio::sync::RwLock<VecDeque<DocumentProcessingRequest>>> =
    Lazy::new(|| RwLock::new(VecDeque::new()));

// shutdown bit, when this is set to true the document processing pipeline will be shut down
static STOP_FLAG: Lazy<tokio::sync::RwLock<bool>> = Lazy::new(|| RwLock::new(false));

// model will only be initialized and stored if there are documents that need processing
static MODEL_SINGLETON: Lazy<tokio::sync::Mutex<Option<LLModelExtractor>>> =
    Lazy::new(|| Mutex::new(None));

use crate::{
    config::Config,
    extract::{LLModelExtractor, ModelError},
    requests,
    types::{FieldError, custom_field_learning_supported},
};

#[derive(Debug, PartialEq, Hash, Clone, Copy, Eq)]
#[non_exhaustive]
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
    overwrite_finshed_tag: Option<Tag>,
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
    ExtractionError(#[from] FieldError),
}

async fn handle_correspondend_suggest(
    doc: &mut Document,
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
    doc.correspondent = Some(new_crrspndt.id);

    // defered sync back to paperless instance
    // after successfull finish the state of document on paperless will be updated by the update task
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
                log::debug!(
                    "Extracted custom field for document {}\n {:#?}",
                    doc.id,
                    cf_value
                );
                doc.custom_fields.as_mut().map(|doc_custom_fields| {
                    for doc_cf_i in doc_custom_fields.iter_mut() {
                        if doc_cf_i.field == cf_value.field {
                            *doc_cf_i = cf_value.clone()
                        }
                    }
                });
            }
        }
    }

    // defered sync back to paperless instance
    // after successfull finish the state of document on paperless will be updated by the update task
    Ok(())
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
/// General webhook parameters any workflow trigger will accept this type
struct WebhookParams {
    /// url of the document that should be processed
    document_url: String,
    #[serde(default)]
    /// tag to apply to document when finished with processing, this is optional if unspecfied the configured finsh tag will be set
    next_tag: Option<String>,
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
                        let mut next_tag = None;
                        if let Some(nt_to_parse) = &self.next_tag {
                            if let Ok(nt_as_id) = nt_to_parse.parse::<i64>() {
                                next_tag = requests::fetch_tag_by_id_or_name(
                                    &mut api_client,
                                    None,
                                    Some(nt_as_id),
                                )
                                .await;
                            } else {
                                next_tag = requests::fetch_tag_by_id_or_name(
                                    &mut api_client,
                                    Some(nt_to_parse.clone()),
                                    None,
                                )
                                .await;
                            }
                        }
                        if next_tag.is_none() && self.next_tag.is_some() {
                            log::warn!(
                                "Webhook received request to use specific finished tag, but the tag does not exists (next_tag=`{}`)! Ignoring tag from request!",
                                self.next_tag.as_ref().unwrap()
                            );
                        }
                        let _ = document_pipeline.send(DocumentProcessingRequest {
                            document: doc,
                            processing_type: req_type,
                            overwrite_finshed_tag: next_tag,
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

#[utoipa::path(tag = "llm_workflow_trigger")]
#[post("/suggest/correspondent")]
/// Workflow to suggest a correspondent for a document
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

#[utoipa::path(tag = "llm_workflow_trigger")]
#[post("/fill/custom_fields")]
/// Workflow to fill unfilled custom fields on a document
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

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(suggest_correspondent, custom_field_prediction),
    components(schemas(WebhookParams))
)]
struct DocumentProcessingApiSpec;

struct DocumentProcessingApi;

impl HttpServiceFactory for DocumentProcessingApi {
    fn register(self, config: &mut actix_web::dev::AppService) {
        custom_field_prediction.register(config);
        suggest_correspondent.register(config);
    }
}

/// given an updated document add changes from processing type to other instances of the document
///
/// the purpose of this function is the following situation, given a document may be present multiple times in the processing queue
/// and with later document versions having potentially more information in them this function should update later versions of the
/// document with data from previously run processing steps without loosing any additional data. The goal being to mininmize the amount
/// of back communication with the paperless server to avoid updating documents multiple times and triggering workflows unnecessarìly
fn merge_document_status(
    doc: &mut Document,
    updated_doc: &Document,
    processing_type: &ProcessingType,
) {
    if doc.id != updated_doc.id {
        // if document ids do not match stop here!
        return;
    }
    match processing_type {
        ProcessingType::CustomFieldPrediction => {
            if let Some(updated_custom_fields) = &updated_doc.custom_fields {
                for updated_cf in updated_custom_fields {
                    doc.custom_fields.as_mut().map(|doc_custom_fields| {
                        let mut cf_found = false;
                        for doc_cf_i in &mut *doc_custom_fields {
                            if doc_cf_i.field == updated_cf.field {
                                cf_found = true;
                                doc_cf_i.value = updated_cf.value.clone()
                            }
                        }
                        if !cf_found {
                            doc_custom_fields.push(updated_cf.clone());
                        }
                    });
                }
            }
        }
        ProcessingType::CorrespondentSuggest => doc.correspondent = updated_doc.correspondent,
    }
}

/// this jobs function is to receive the processed and send the results back the paperless instance
///
/// the goal is to minimize traffic to the paperless instance and avoid waiting for api requests
async fn document_updater(
    status_tags: PaperlessStatusTags,
    mut api_client: Client,
    mut document_update_channel: tokio::sync::mpsc::UnboundedReceiver<
        Result<
            (DocumentProcessingRequest, bool),
            (DocumentProcessingError, DocumentProcessingRequest, bool),
        >,
    >,
) {
    let mut defered_doc_updates: BTreeMap<i64, Vec<ProcessingType>> = BTreeMap::new();

    while let Some(doc_update) = document_update_channel.recv().await {
        let mut _maybe_error = None;
        let same_doc_in_queue_again;
        let doc_req = match doc_update {
            Ok((doc_req, queued_again)) => {
                same_doc_in_queue_again = queued_again;
                doc_req
            }
            Err((err, doc_req, queued_again)) => {
                same_doc_in_queue_again = queued_again;
                _maybe_error = Some(err);
                doc_req
            }
        };

        // if there are now further processing steps pending for the document then it's
        // state can be synced back to the paperless server and all processing tags removed
        // finshed / next tag will be set
        if !same_doc_in_queue_again {
            let updated_doc_tags: Vec<i64> = doc_req
                .document
                .tags
                .iter()
                .map(|t| *t)
                .filter(|t| *t != status_tags.processing.id)
                .chain(if doc_req.overwrite_finshed_tag.is_none() {
                    [status_tags.finished.id].into_iter()
                } else {
                    [doc_req.overwrite_finshed_tag.as_ref().unwrap().id].into_iter()
                })
                .unique()
                .collect();

            let mut updated_cf: Option<Vec<CustomFieldInstance>> = None;
            let mut updated_crrspdnt: Option<i64> = None;

            for doc_processing_steps in vec![doc_req.processing_type]
                .iter()
                .chain(
                    defered_doc_updates
                        .get(&doc_req.document.id)
                        .unwrap_or(&Vec::new()),
                )
                .unique()
            {
                match doc_processing_steps {
                    ProcessingType::CustomFieldPrediction => {
                        if let Some(cfis) = doc_req.document.custom_fields.as_ref() {
                            updated_cf = Some(cfis.clone());
                        }
                    }
                    ProcessingType::CorrespondentSuggest => {
                        updated_crrspdnt = doc_req.document.correspondent;
                    }
                }
            }

            let _ = requests::processed_doc_update(
                &mut api_client,
                doc_req.document.id,
                updated_doc_tags,
                updated_crrspdnt,
                updated_cf,
            )
            .await
            .map_err(|err| {
                log::error!("{err}");
                err
            });
        } else {
            // remember how document has been processed until now for defered update later
            if defered_doc_updates.contains_key(&doc_req.document.id) {
                if defered_doc_updates
                    .get(&doc_req.document.id)
                    .is_some_and(|v| v.contains(&doc_req.processing_type))
                {
                    continue;
                } else {
                    if let Some(v) = defered_doc_updates.get_mut(&doc_req.document.id).as_mut() {
                        v.push(doc_req.processing_type);
                    }
                }
            } else {
                defered_doc_updates.insert(doc_req.document.id, vec![doc_req.processing_type]);
            }
        }
    }
}

// future performance optimization needs to focus on this function, it should dispatch to batch processing of documents
// or could combine requests to the same document in the queue.
/// this jobs functions it to batch process documents using the llm and send the results on to the update handler
async fn document_processor(
    config: Config,
    mut api_client: Client,
    document_update_channel: tokio::sync::mpsc::UnboundedSender<
        Result<
            (DocumentProcessingRequest, bool),
            (DocumentProcessingError, DocumentProcessingRequest, bool),
        >,
    >,
) {
    while !*STOP_FLAG.read().await {
        while PROCESSING_QUEUE.read().await.len() > 0 {
            let model_path = config.model.clone();
            {
                let mut model_singleton = MODEL_SINGLETON.lock().await;
                if model_singleton.is_none() {
                    *model_singleton = spawn_blocking(move || {
                        LLModelExtractor::new(&Path::new(&model_path), config.num_gpu_layers, None)
                    })
                    .await
                    .map_err(|err| {
                        log::error!("Error loading Model! {err}");
                        ModelError::ModelNotLoaded
                    })
                    .and_then(|r| r)
                    .ok();
                }
            }
            let mut doc_process_req = {
                // nesting here to ensure write lock is dropped while processing the document in the next step
                PROCESSING_QUEUE
                    .write()
                    .await
                    .pop_front()
                    .expect("Size is greater 0 so there must be a document in the queue")
            };

            let processing_result = match doc_process_req.processing_type {
                ProcessingType::CustomFieldPrediction => {
                    handle_custom_field_prediction(&mut doc_process_req.document, &mut api_client)
                        .await
                }
                ProcessingType::CorrespondentSuggest => {
                    handle_correspondend_suggest(&mut doc_process_req.document, &mut api_client)
                        .await
                }
            };

            let mut doc_in_queue_again = false;
            match processing_result {
                Ok(_) => {
                    // if the same document has more processing requests pending update the doc state to
                    // also contain the newly added data.
                    for next_process_req in PROCESSING_QUEUE.write().await.iter_mut() {
                        if next_process_req.document.id == doc_process_req.document.id {
                            doc_in_queue_again = true;
                            merge_document_status(
                                &mut next_process_req.document,
                                &doc_process_req.document,
                                &doc_process_req.processing_type,
                            );
                        }
                    }
                    let _ = document_update_channel.send(Ok((doc_process_req, doc_in_queue_again)));
                }
                Err(err) => {
                    doc_in_queue_again = PROCESSING_QUEUE
                        .read()
                        .await
                        .iter()
                        .map(|doc_req| doc_req.document.id)
                        .contains(&doc_process_req.document.id);
                    let _ = document_update_channel.send(Err((
                        err,
                        doc_process_req,
                        doc_in_queue_again,
                    )));
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        if PROCESSING_QUEUE.read().await.len() == 0 && MODEL_SINGLETON.lock().await.is_some() {
            // No Documents need processing drop model
            log::info!("Unloading Model due to processing queue being empty!");
            let mut model_singleton = MODEL_SINGLETON.lock().await;
            let _ = model_singleton.take();
        }
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
    let (tx_update, rx_update) = tokio::sync::mpsc::unbounded_channel();

    let status_tags = PaperlessStatusTags {
        processing: processing_tag,
        finished: finished_tag,
    };

    let doc_to_process_queue = spawn(document_request_funnel(rx));
    let doc_processor = spawn(document_processor(
        config.clone(),
        paperless_api_client.clone(),
        tx_update,
    ));
    let doc_update_task = spawn(document_updater(
        status_tags.clone(),
        paperless_api_client.clone(),
        rx_update,
    ));
    let webhook_server = HttpServer::new(move || {
        let app = App::new()
            .app_data(Data::new(tx.clone()))
            .app_data(Data::new(config.clone()))
            .app_data(Data::new(paperless_api_client.clone()))
            .app_data(Data::new(status_tags.clone()))
            .service(
                SwaggerUi::new("/api/{_:.*}")
                    .config(utoipa_swagger_ui::Config::default().use_base_layout())
                    .url("/docs/openapi.json", DocumentProcessingApiSpec::openapi()),
            )
            .service(DocumentProcessingApi);
        app
    })
    .bind(("0.0.0.0", 8123))?
    .run();

    let _ = join!(
        webhook_server,
        doc_to_process_queue,
        doc_processor,
        doc_update_task
    );

    Ok(())
}
