use std::process::exit;

use config::Config;
use extract::CustomFieldModelExtractor;
use paperless_api_client::{
    Client,
    types::{SetPermissions, TagRequest, error},
};
use types::schema_from_custom_field;

mod config;
mod extract;
mod requests;
mod types;

#[tokio::main]
async fn main() {
    colog::init();

    let config = Config::default();

    let mut api_client = Client::new_from_env();
    api_client.set_base_url("https://judge-paperless.16.wlandt.de");

    let custom_fields = requests::get_all_custom_fields(&mut api_client).await;
    let tags = requests::get_all_tags(&mut api_client).await;
    let users = requests::get_all_users(&mut api_client).await;

    println!("{tags:#?}");
    exit(1);

    //make sure tags for processing and finshed exists
    let (user, processing_tag) = if tags
        .iter()
        .filter(|t| t.name == config.processing_tag)
        .next()
        .is_none()
    {
        if let Some(user) = users
            .iter()
            .filter(|user| user.username == config.tag_user_name)
            .next()
        {
            let processing_tag = requests::create_tag(
                &mut api_client,
                user,
                &config.processing_tag,
                &config.processing_color,
            )
            .await
            .map_err(|err| {
                log::error!("could not create processing tag: {err}");
                err
            })
            .map(|tag| {
                log::info!(
                    "created processing tag `{}` to paperless ",
                    config.processing_tag
                );
                tag
            })
            .ok();
            (user, processing_tag)
        } else {
            log::error!(
                "Can not create any tags, because configured user `{}` does not exists in paperless!",
                config.tag_user_name
            );
            exit(1)
        }
    } else {
        todo!()
    };

    let finished_tag = if tags
        .iter()
        .filter(|t| t.name == config.finished_tag)
        .next()
        .is_none()
    {
        requests::create_tag(
            &mut api_client,
            user,
            &config.finished_tag,
            &config.finished_color,
        )
        .await
        .map_err(|err| {
            log::error!("could not create finished tag: {err}");
            err
        })
        .map(|tag| {
            log::info!(
                "created processing tag `{}` to paperless ",
                config.processing_tag
            );
            tag
        })
        .ok()
    } else {
        todo!()
    };

    let mut docs_with_empty_custom_fields = requests::get_all_docs(&mut api_client).await;

    //println!("{:#?}", docs_with_empty_custom_fields);
    //exit(1);
    // find document with empty custom fields
    docs_with_empty_custom_fields = docs_with_empty_custom_fields
        .into_iter()
        .filter(|d| {
            let mut has_inbox_tag = false;
            for t in &d.tags {
                if let Some(tag_def) = tags
                    .iter()
                    .filter(|td| td.id == *t)
                    .collect::<Vec<_>>()
                    .first()
                {
                    has_inbox_tag = tag_def.is_inbox_tag.is_some_and(|v| v);
                    break;
                }
            }
            has_inbox_tag
        })
        .filter(|d| {
            d.custom_fields.as_ref().is_some_and(|cfields| {
                !cfields
                    .iter()
                    .filter(|f| f.value.is_none())
                    .collect::<Vec<_>>()
                    .is_empty()
            })
        })
        .collect();

    for cf in custom_fields {
        log::info!("Processing documents with empty {} custom fields.", cf.name);
        let docs_to_process = docs_with_empty_custom_fields
            .iter()
            .filter(|doc| {
                doc.custom_fields.as_ref().is_some_and(|doc_cf| {
                    !doc_cf
                        .iter()
                        .filter(|cf_instance| {
                            cf_instance.field == cf.id && cf_instance.value.is_none()
                        })
                        .collect::<Vec<_>>()
                        .is_empty()
                })
            })
            .collect::<Vec<_>>();
        if docs_to_process.is_empty() {
            continue;
        }
        if let Some(field_grammar) = schema_from_custom_field(&cf) {
            let mut model_extractor = CustomFieldModelExtractor::new(&field_grammar);
            for doc in docs_to_process {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&model_extractor.extract(doc)).unwrap()
                );
            }
        }
    }
}
