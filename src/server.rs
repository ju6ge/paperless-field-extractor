use std::{collections::VecDeque, sync::Arc};

use actix_web::{
    App, HttpResponse, HttpResponseBuilder, HttpServer, ResponseError,
    dev::{HttpServiceFactory, Url},
    error::PayloadError,
    get,
    http::{StatusCode, Uri, uri},
    post,
    web::{self, Data},
};
use once_cell::sync::Lazy;
use paperless_api_client::{
    Client,
    types::{Document, Tag},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{join, spawn, sync::RwLock, task::spawn_blocking};

static DOCID_REGEX: Lazy<Regex> = regex_static::lazy_regex!(r"documents/(\d*)");

static PROCESSING_QUEUE: Lazy<tokio::sync::RwLock<VecDeque<DocumentProcessingRequest>>> = Lazy::new(|| {
    RwLock::new(VecDeque::new())
});

use crate::{
    config::{self, Config},
    requests,
};

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
                            let _ = requests::update_document_tag_ids(&mut api_client, &mut doc, &updated_doc_tags).await;
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

async fn document_processor(status_tags: PaperlessStatusTags) {
    todo!()
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

    let doc_processor = spawn(document_processor(status_tags.clone()));
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
