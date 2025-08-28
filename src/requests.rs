use futures::StreamExt;
use log::{error, info};
use paperless_api_client::{
    Client,
    types::{CustomField, Document, Tag},
};

pub async fn get_all_custom_fields(client: &mut Client) -> Vec<CustomField> {
    info!("Requesting Custom Fields from Server");
    client
        .custom_fields()
        .list_stream(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .filter_map(async |cf_result| {
            cf_result
                .map_err(|err| {
                    error!("{err}");
                    err
                })
                .ok()
        })
        .collect()
        .await
}

pub async fn get_all_tags(client: &mut Client) -> Vec<Tag> {
    info!("Requesting All Tags from Server");
    client
        .tags()
        .list_stream(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .filter_map(async |tag_result| {
            tag_result
                .map_err(|err| {
                    error!("{err}");
                    err
                })
                .ok()
        })
        .collect()
        .await
}

pub async fn get_all_docs(client: &mut Client) -> Vec<Document> {
    client
        .documents()
        .list_stream(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .filter_map(async |doc_request| {
            doc_request
                .map_err(|err| {
                    error!("{err}");
                    err
                })
                .ok()
        })
        .collect()
        .await
}
