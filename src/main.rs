use std::fs::File;

use paperless_api_client::{types::Document, Client};

mod requests;



#[tokio::main]
async fn main() {
    colog::init();
    let mut api_client = Client::new_from_env();
    api_client.set_base_url("https://judge-paperless.16.wlandt.de");

    let c_fields = requests::get_all_custom_fields(&mut api_client).await;
    let tags = requests::get_all_tags(&mut api_client).await;
    let mut docs = requests::get_all_docs(&mut api_client).await;

    docs = docs.into_iter().filter(|d| {
        let mut has_inbox_tag = false;
        for t in &d.tags {
            if let Some(tag_def) = tags.iter().filter(|td| td.id == *t).collect::<Vec<_>>().first() {
                has_inbox_tag = tag_def.is_inbox_tag.is_some_and(|v| v)
            }
        }
        has_inbox_tag
    }).collect();

    println!("{docs:#?}")
}
