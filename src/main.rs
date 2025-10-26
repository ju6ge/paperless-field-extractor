use std::{path::Path, process::exit};

use config::{Config, OverlayConfig};
use extract::CustomFieldModelExtractor;
use paperless_api_client::Client;
use types::{custom_field_learning_supported, schema_from_custom_field};

mod config;
mod extract;
mod requests;
mod types;

#[cfg(all(feature = "vulkan", feature = "native"))]
compile_error!("Only one compute backend can be used, choose feature `vulkan` or `native`!");

#[cfg(not(any(feature = "vulkan", feature = "native")))]
compile_error!("Choose feature `vulkan` or `native` to select what compute backend should be used for inference!");

#[tokio::main]
async fn main() {
    colog::init();

    let config = Config::default()
        .overlay_config(OverlayConfig::read_config_toml(Path::new("./config.toml")));

    let model_path = Path::new(&config.model)
        .canonicalize()
        .map_err(|err| {
            log::error!(
                "Could not find model file! Can not run without a language model! … Stop execution!"
            );
            err
        })
        .unwrap();

    let mut api_client = Client::new_from_env();
    api_client.set_base_url("https://judge-paperless.16.wlandt.de");

    let custom_fields = requests::get_all_custom_fields(&mut api_client)
        .await
        .into_iter()
        .filter(|cf| {
            let learing_supported = custom_field_learning_supported(&cf);
            if !learing_supported {
                log::warn!("Custom Fields with name `{}` are using an unsupported custom field type, will be ignored!", cf.name)
            }
            learing_supported
        })
        .collect::<Vec<_>>();
    let tags = requests::get_all_tags(&mut api_client).await;
    let users = requests::get_all_users(&mut api_client).await;

    let user = users
        .iter()
        .filter(|user| user.username == config.tag_user_name)
        .next()
        .or_else(|| {
            log::warn!(
                "configured user `{}` could not be found, running without user!",
                config.tag_user_name
            );
            None
        });

    //make sure tags for processing and finshed exists
    let processing_tag = if tags
        .iter()
        .filter(|t| t.name == config.processing_tag)
        .next()
        .is_none()
    {
        requests::create_tag(
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
        .ok()
    } else {
        tags.iter()
            .filter(|t| t.name == config.processing_tag)
            .next()
            .map(|t| t.clone())
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
                config.finished_tag
            );
            tag
        })
        .ok()
    } else {
        tags.iter()
            .filter(|t| t.name == config.finished_tag)
            .next()
            .map(|t| t.clone())
    };

    if processing_tag.is_none() || finished_tag.is_none() {
        log::error!("Processing and Finshed Tags could neither be found nor created! Exiting …");
        exit(1);
    }

    let processing_tag = processing_tag.unwrap();
    let finished_tag = finished_tag.unwrap();

    // find document with empty custom fields
    let mut docs_with_empty_custom_fields = requests::get_all_docs(&mut api_client)
        .await
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
                    if has_inbox_tag {
                        break;
                    }
                }
            }
            has_inbox_tag
        })
        .filter(|d| {
            d.custom_fields.as_ref().is_some_and(|cfields| {
                !cfields
                    .iter()
                    // only check for empty fields for supported custom field types
                    .filter(|f| {
                        custom_fields
                            .iter()
                            .map(|cf| cf.id)
                            .find(|id| *id == f.field)
                            .is_some()
                    })
                    .filter(|f| f.value.is_none())
                    .collect::<Vec<_>>()
                    .is_empty()
            })
        })
        .collect::<Vec<_>>();

    // add processing tag to all documents with unpopulated custom fields
    for doc in &mut docs_with_empty_custom_fields {
        let mut current_doc_tags = tags
            .iter()
            .filter(|tag| doc.tags.contains(&tag.id))
            .collect::<Vec<_>>();
        // if the current document tags contain the processing tag already skip to the next document
        if current_doc_tags
            .iter()
            .filter(|t| t.id == processing_tag.id)
            .next()
            .is_some()
        {
            continue;
        }
        current_doc_tags.push(&processing_tag);
        let _ = requests::update_document_tags(&mut api_client, doc, &current_doc_tags)
            .await
            .map(|_| {
                log::debug!("Added processing tag to document with id {}", doc.id);
            })
            .map_err(|err| {
                log::warn!(
                    "Could not add processing tag to document with id {}: {err}",
                    doc.id
                );
                err
            });
    }

    //eprintln!("{docs_with_empty_custom_fields:#?}");

    for cf in &custom_fields {
        let docs_to_process = docs_with_empty_custom_fields
            .iter_mut()
            .filter(|doc| {
                doc.custom_fields.as_ref().is_some_and(|doc_cf| {
                    !doc_cf
                        .iter()
                        // only check for empty fields for supported custom field types
                        .filter(|f| {
                            custom_fields
                                .iter()
                                .map(|cf| cf.id)
                                .find(|id| *id == f.field)
                                .is_some()
                        })
                        .filter(|cf_instance| {
                            cf_instance.field == cf.id && cf_instance.value.is_none()
                        })
                        .collect::<Vec<_>>()
                        .is_empty()
                })
            })
            .collect::<Vec<_>>();
        log::info!(
            "Processing documents with empty {} custom fields … Documents to process {}",
            cf.name,
            docs_to_process.len()
        );
        if docs_to_process.is_empty() {
            continue;
        }
        if let Some(field_grammar) = schema_from_custom_field(&cf) {
            let mut model_extractor = CustomFieldModelExtractor::new(
                model_path.as_path(),
                config.num_gpu_layers,
                &field_grammar,
            );
            for doc in docs_to_process {
                let extracted_custom_field_data = model_extractor.extract(doc);
                if let Ok(cf_inst) = extracted_custom_field_data.to_custom_field_instance(&cf) {
                    // need to get at the custom field list with the following if let
                    // since we are only processing documnts with custom fields this will always be some
                    // list of custom fields
                    if let Some(doc_custom_fields) = &mut doc.custom_fields {
                        let updated_custom_fields = doc_custom_fields
                            .iter_mut()
                            .map(|doc_cf| {
                                if doc_cf.field == cf_inst.field {
                                    *doc_cf = cf_inst.clone()
                                }
                                doc_cf
                            })
                            .map(|cf| cf.clone())
                            .collect::<Vec<_>>();
                        // send extracted custom field to server and update document
                        //let _ = requests::update_document_custom_fields(
                            //&mut api_client,
                            //doc,
                            //updated_custom_fields.as_slice(),
                        //)
                        //.await
                        //.map(|_| {
                            //log::info!(
                                //"Updated custom field {} for document with id {}",
                                //cf.name,
                                //doc.id
                            //);
                        //})
                        //.map_err(|err| {
                            //log::error!(
                                //"Error updating custom field {} on document with id {}! \n {err}",
                                //cf.name,
                                //doc.id
                            //);
                            //err
                        //});
                    }
                    // if all custom fields have been filled then updated tag to indicate finished status
                    if doc
                        .custom_fields
                        .as_ref()
                        .is_some_and(|custom_fields_instances| {
                            custom_fields_instances
                                .iter()
                                // only check for empty fields for supported custom field types
                                .filter(|f| {
                                    custom_fields
                                        .iter()
                                        .map(|cf| cf.id)
                                        .find(|id| *id == f.field)
                                        .is_some()
                                })
                                .filter(|cf| cf.value.is_none()) // if there are any custom fields with empty values
                                .next() // then processing for this document has not finished
                                .is_none()
                        })
                    {
                        let mut current_doc_tags = tags
                            .iter()
                            .filter(|tag| doc.tags.contains(&tag.id))
                            .filter(|tag| tag.name != config.processing_tag) // remove processing tag from document
                            .collect::<Vec<_>>();
                        current_doc_tags.push(&finished_tag);
                        let _ =
                            requests::update_document_tags(&mut api_client, doc, &current_doc_tags)
                                .await
                                .map(|_| {
                                    log::debug!(
                                        "Added finished tag to document with id {}",
                                        doc.id
                                    );
                                })
                                .map_err(|err| {
                                    log::warn!(
                                        "Could not add finished tag to document with id {}: {err}",
                                        doc.id
                                    );
                                    err
                                });
                    }
                }
            }
        }
    }
}
