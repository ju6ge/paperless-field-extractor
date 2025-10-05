use paperless_api_client::Client;
use types::schema_from_custom_field;

mod types;
//mod extract;
mod requests;

async fn fetch_stuff_from_paperless(api_client: &mut Client) {
    let c_fields = requests::get_all_custom_fields(api_client).await;
    let tags = requests::get_all_tags(api_client).await;
    let mut docs = requests::get_all_docs(api_client).await;

    for cf in c_fields {
        println!(
            "{}",
            serde_json::to_string_pretty(schema_from_custom_field(cf).unwrap().as_value()).unwrap()
        )
    }

    docs = docs
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
        .collect();

    println!("{}", docs.len())
}

#[tokio::main]
async fn main() {
    colog::init();

    let mut api_client = Client::new_from_env();
    api_client.set_base_url("https://judge-paperless.16.wlandt.de");

    fetch_stuff_from_paperless(&mut api_client).await;
}
