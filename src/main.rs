use std::{path::Path, process::exit};

use clap::Parser;
use config::{Config, OverlayConfig};
use extract::CustomFieldModelExtractor;
use paperless_api_client::Client;
use types::{
    custom_field_learning_supported, schema_from_correspondents, schema_from_custom_field,
};

mod config;
mod extract;
mod requests;
mod types;

#[cfg(any(
    all(feature = "vulkan", feature = "native"),
    all(feature = "vulkan", feature = "openmp"),
    all(feature = "vulkan", feature = "cuda"),
    all(feature = "openmp", feature = "cuda"),
    all(feature = "openmp", feature = "native"),
    all(feature = "cuda", feature = "native")
))]
compile_error!(
    "Only one compute backend can be used, choose feature `vulkan`, `openmp`, `cuda` or `native`!"
);

#[cfg(not(any(
    feature = "vulkan",
    feature = "native",
    feature = "openmp",
    feature = "cuda"
)))]
compile_error!(
    "Choose feature `vulkan`, `openmp`, `cuda` or `native` to select what compute backend should be used for inference!"
);

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, default_value_t = false, action)]
    dry_run: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    colog::init();

    let config = Config::default()
        .overlay_config(OverlayConfig::read_config_toml(Path::new(
            "/etc/paperless-field-extractor/config.toml",
        )))
        .overlay_config(OverlayConfig::read_from_env());

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
    api_client.set_base_url(config.paperless_server);

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

    if !args.dry_run {}
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

    let mut all_inbox_docs: Vec<_> = requests::get_all_docs(&mut api_client)
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
        .collect();

    if config.correspondent_suggestions {
        let crrspndts = requests::fetch_all_correspondents(&mut api_client).await;
        let crrspndts_suggest_schema = schema_from_correspondents(crrspndts.as_slice());

        let mut model_extractor = CustomFieldModelExtractor::new(
            model_path.as_path(),
            config.num_gpu_layers,
            &crrspndts_suggest_schema,
            None,
        );

        for doc in &all_inbox_docs {
            if args.dry_run {
                println!(
                    "================================================================================"
                );
                println!(
                    "Current correspondent {:?}",
                    crrspndts
                        .iter()
                        .filter(|c| doc.correspondent.is_some_and(|cr| c.id == cr))
                        .map(|c| &c.name)
                        .next()
                );
                println!("========= Content ==========");
                println!(
                    "{}",
                    doc.content
                        .as_ref()
                        .map(|s| s.split_at(std::cmp::min(1024, s.len())).0)
                        .unwrap_or("")
                );
            }
            if let Ok(extracted_correspondent) = model_extractor
                .extract(&serde_json::to_value(&doc.content).unwrap(), args.dry_run)
                .map_err(|err| {
                    log::error!("{err}");
                    err
                })
            {
                if args.dry_run {
                    println!(
                        "Predicted Correspondent: {}",
                        serde_json::to_string_pretty(&extracted_correspondent.value).unwrap()
                    );
                }

                if let Ok(predicted_correspondent) =
                    extracted_correspondent.to_correspondent(&crrspndts.as_slice())
                {
                    if doc
                        .correspondent
                        .is_some_and(|dc| dc == predicted_correspondent.id)
                    {
                        // predicted correspondent is the same as the one set on the document already, nothing to do
                        continue;
                    }
                    if let Some(doc_suggestions) =
                        requests::fetch_doc_suggestions(&mut api_client, &doc).await
                    {
                        if doc_suggestions
                            .correspondents
                            .contains(&predicted_correspondent.id)
                        {
                            // predicted correspondent is already present in the other suggestions for the document, nothing to do
                            continue;
                        }
                    }
                    // only if the document has no correspondent and no suggestion contains the predicted correspondent update
                    // the correspondent field of the document, sadly directly editing suggestions is not possible via the api
                    // so here we have to overwrite the correspondent that was predicted previously
                    if !args.dry_run {
                        let _ = requests::update_doc_correspondent(
                            &mut api_client,
                            &doc,
                            &predicted_correspondent,
                        )
                        .await
                        .map(|_| log::info!("Updated Correspondet for Document with id {}", doc.id))
                        .map_err(|err| log::error!("{err}"));
                    }
                } else {
                    // seems the predicted correspondent does not exists, this should be impossible due to grammar
                    continue;
                }
            }
        }
    }

    // find document with empty custom fields
    let mut docs_with_empty_custom_fields = all_inbox_docs
        .into_iter()
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
            || args.dry_run
        // do not set any tags when doing a dry run!
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
                None,
            );
            for doc in docs_to_process {
                if let Ok(extracted_custom_field_data) = model_extractor
                    .extract(&serde_json::to_value(&doc).unwrap(), args.dry_run)
                    .map_err(|err| {
                        log::error!("{err}");
                        err
                    })
                {
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
                            if !args.dry_run {
                                let _ = requests::update_document_custom_fields(
                                &mut api_client,
                                doc,
                                updated_custom_fields.as_slice(),
                            )
                            .await
                            .map(|_| {
                                log::info!(
                                    "Updated custom field {} for document with id {}",
                                    cf.name,
                                    doc.id
                                );
                            })
                            .map_err(|err| {
                                log::error!(
                                    "Error updating custom field {} on document with id {}! \n {err}",
                                    cf.name,
                                    doc.id
                                );
                                err
                            });
                            }
                        }
                    }
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
                    && !args.dry_run
                {
                    let mut current_doc_tags = tags
                        .iter()
                        .filter(|tag| doc.tags.contains(&tag.id))
                        .filter(|tag| tag.name != config.processing_tag) // remove processing tag from document
                        .collect::<Vec<_>>();
                    current_doc_tags.push(&finished_tag);
                    let _ = requests::update_document_tags(&mut api_client, doc, &current_doc_tags)
                        .await
                        .map(|_| {
                            log::debug!("Added finished tag to document with id {}", doc.id);
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
