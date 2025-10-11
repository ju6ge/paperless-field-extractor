use extract::CustomFieldModelExtractor;
use paperless_api_client::Client;
use types::schema_from_custom_field;

mod extract;
mod requests;
mod types;

#[tokio::main]
async fn main() {
    colog::init();

    let mut api_client = Client::new_from_env();
    api_client.set_base_url("https://judge-paperless.16.wlandt.de");

    let custom_fields = requests::get_all_custom_fields(&mut api_client).await;
    let tags = requests::get_all_tags(&mut api_client).await;
    let mut docs_with_empty_custom_fields = requests::get_all_docs(&mut api_client).await;
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
            println!("{}", serde_json::to_string_pretty(&field_grammar).unwrap());
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
