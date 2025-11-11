use actix_web::{
    dev::{HttpServiceFactory, Url}, error::PayloadError, get, http::{uri, Uri}, post, web::{self, Data}, App, HttpResponse, HttpResponseBuilder, HttpServer
};
use paperless_api_client::{Client, types::Tag};
use serde::{Deserialize, Serialize};
use tokio::{join, spawn, task::spawn_blocking};

use crate::config::{self, Config};

#[non_exhaustive]
enum ProcessingType {
    CustomFieldPrediction,
    CorrespondentSuggest,
}

/// Most processing of document will involve feeding document data throuh a large languae model.
/// Since LLM are notoriously resource intensive a task queue is used in order to facilitate asyncronous
/// batched processinsg a container is required to hold the queue of processing requests
struct DocumentProcessingRequest {
    document_id: i64,
    processing_type: ProcessingType,
}

#[get("/fill/custom_fields")]
async fn custom_field_prediction(
    params: web::Json<WebhookParams>,
    status_tags: Data<PaperlessStatusTags>,
    api_client: Data<Client>,
    config: Data<Config>,
    document_pipeline: web::Data<tokio::sync::mpsc::UnboundedSender<DocumentProcessingRequest>>,
) -> HttpResponse {
    let doc_url: Uri = params.document_url.parse().unwrap();
    let configured_paperless_server: Uri = config.paperless_server.parse().unwrap();
    // verify document host and configured paperless instance are the same host to avoid handling requests from other paperless instances
    if doc_url.host().is_some_and(|rs| configured_paperless_server.host().is_some_and(|cs| cs == rs)) {
        HttpResponse::Accepted().into()
    } else {
        HttpResponse::Unauthorized().into()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WebhookParams {
    document_url: String
}

#[post("/suggest/correspondent")]
async fn suggest_correspondent(
    params: web::Json<WebhookParams>,
    status_tags: Data<PaperlessStatusTags>,
    api_client: Data<Client>,
    config: Data<Config>,
    document_pipeline: web::Data<tokio::sync::mpsc::UnboundedSender<DocumentProcessingRequest>>,
) -> HttpResponse {
    let doc_url: Uri = params.document_url.parse().unwrap();
    let configured_paperless_server: Uri = config.paperless_server.parse().unwrap();
    // verify document host and configured paperless instance are the same host to avoid handling requests from other paperless instances
    if doc_url.host().is_some_and(|rs| configured_paperless_server.host().is_some_and(|cs| cs == rs)) {
        HttpResponse::Accepted().into()
    } else {
        HttpResponse::Unauthorized().into()
    }
}

struct DocumentProcessingApi;

impl HttpServiceFactory for DocumentProcessingApi {
    fn register(self, config: &mut actix_web::dev::AppService) {
        custom_field_prediction.register(config);
        suggest_correspondent.register(config);
    }
}

async fn document_processor(
    mut processing_request_channel: tokio::sync::mpsc::UnboundedReceiver<DocumentProcessingRequest>,
    status_tags: PaperlessStatusTags
) {
    while let Some(prc_req) = processing_request_channel.recv().await {
        println!("Received Request for Document {}", prc_req.document_id);
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

    let doc_processor = spawn(document_processor(rx, status_tags.clone()));
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

    join!(webhook_server, doc_processor);

    Ok(())
}
