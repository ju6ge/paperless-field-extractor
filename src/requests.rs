use futures::StreamExt;
use log::{error, info};
use paperless_api_client::{
    types::{
        CustomField, CustomFieldInstance, CustomFieldInstanceRequest, Document, DocumentRequest, PatchedDocumentRequest, Tag, TagRequest, User
    }, Client
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
    tag_user: Option<&User>,
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
            owner: tag_user.map(|u| u.id),
            set_permissions: None,
        })
        .await
}

pub(crate) async fn update_document_tags(
    api_client: &mut Client,
    doc: &mut Document,
    tags: &[&Tag],
) -> Result<(), paperless_api_client::types::error::Error> {
    *doc = api_client
        .documents()
        .partial_update(
            doc.id,
            &PatchedDocumentRequest {
                tags: Some(tags.iter().map(|t| t.id).collect()),
                correspondent: Default::default(),
                document_type: Default::default(),
                storage_path: Default::default(),
                title: Default::default(),
                content: Default::default(),
                created: Default::default(),
                created_date: Default::default(),
                deleted_at: Default::default(),
                archive_serial_number: Default::default(),
                owner: Default::default(),
                set_permissions: Default::default(),
                custom_fields: Default::default(),
                remove_inbox_tags: Default::default(),
            },
        )
        .await?;
    Ok(())
}

pub(crate) async fn update_document_custom_fields(
    api_client: &mut Client,
    doc: &mut Document,
    custom_fields: &[CustomFieldInstance],
) -> Result<(), paperless_api_client::types::error::Error> {
    *doc = api_client
        .documents()
        .partial_update(
            doc.id,
            &PatchedDocumentRequest {
                custom_fields: Some(custom_fields.iter().map(|cf| {
                    CustomFieldInstanceRequest {
                        value: cf.value.clone(),
                        field: cf.field,
                    }
                }).collect()),
                tags: Default::default(),
                correspondent: Default::default(),
                document_type: Default::default(),
                storage_path: Default::default(),
                title: Default::default(),
                content: Default::default(),
                created: Default::default(),
                created_date: Default::default(),
                deleted_at: Default::default(),
                archive_serial_number: Default::default(),
                owner: Default::default(),
                set_permissions: Default::default(),
                remove_inbox_tags: Default::default(),
            },
        )
        .await?;
    Ok(())
}
