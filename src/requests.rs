use futures::StreamExt;
use log::{error, info};
use paperless_api_client::{
    Client,
    types::{CustomField, Document, Tag, TagRequest, User},
};

use crate::config;

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

pub(crate) async fn get_all_users(api_client: &mut Client) -> Vec<User> {
    api_client
        .users()
        .list_stream(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .filter_map(async |user_request| {
            user_request
                .map_err(|err| {
                    error!("{err}");
                    err
                })
                .ok()
        })
        .collect()
        .await
}

pub(crate) async fn create_tag(
    api_client: &mut Client,
    tag_user: &User,
    tag_name: &String,
    tag_color: &String,
) -> Result<Tag, paperless_api_client::types::error::Error> {
    api_client
        .tags()
        .create(&TagRequest {
            name: tag_name.clone(),
            color: Some(tag_color.clone()),
            match_: Some("".to_string()),
            matching_algorithm: Some(0),
            is_insensitive: Some(true),
            is_inbox_tag: Some(false),
            owner: Some(tag_user.id),
            set_permissions: None,
        })
        .await
}
