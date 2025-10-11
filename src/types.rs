use paperless_api_client::types::CustomField;
use schemars::{JsonSchema, json_schema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, JsonSchema)]
pub(crate) struct FieldExtract {
    /// this field is used to guide the model to extract the desired data
    /// during grammar generation the string will be set to a constant value
    /// with the content being the name of the custom field that is to be extracted
    field_description: String,
    /// since the custom field can hold any kind of data a generic json value is required to
    /// to hold it. During grammar generation the type of this value will be replaced with
    /// the type of the custom field
    field_value: Value,
    /// as with `field_value` the element type will be replaced during grammar generation
    /// to correspond to the type of the custom field. This field currently does not really
    /// do much, the idea would be to use it in training for reward models if output the correct field
    /// here although it might have put the wrong value in the actual value field. Not sure if this makes
    /// sense though. Another idea would be to add these as suggestions to paperless, but for that to work
    /// custom field suggestions would need to be implemented for paperless first. I don't think they are
    /// at the moment
    alternative_values: Vec<Value>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SelectOption {
    id: String,
    label: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct FieldSelect {
    select_options: Vec<SelectOption>,
}

pub(crate) fn schema_from_custom_field(cf: &CustomField) -> Option<schemars::Schema> {
    let mut base_schema = schema_for!(FieldExtract);
    // set field of description schema as a constant string value matching the name
    // of the custom field. This should guide the llm token generation to extract the
    // desired information from the document
    base_schema.get_mut("properties").map(|properties| {
        properties
            .get_mut("field_description")
            .map(|description_schema| {
                *description_schema = json_schema!({ "const": cf.name }).as_value().clone()
            });
        properties
    });
    let field_schema = match cf.data_type {
        paperless_api_client::types::DataTypeEnum::String => schema_for!(String),
        paperless_api_client::types::DataTypeEnum::Date => schema_for!(chrono::NaiveDate),
        paperless_api_client::types::DataTypeEnum::Boolean => schema_for!(bool),
        paperless_api_client::types::DataTypeEnum::Integer => schema_for!(i64),
        paperless_api_client::types::DataTypeEnum::Float => schema_for!(f64),
        paperless_api_client::types::DataTypeEnum::Monetary => schema_for!(f64),
        paperless_api_client::types::DataTypeEnum::Select => {
            let select_options: FieldSelect = if let Some(v) = &cf.extra_data {
                serde_json::from_value(v.clone()).unwrap()
            } else {
                return None;
            };
            let enum_values = serde_json::to_value(
                &select_options
                    .select_options
                    .into_iter()
                    .map(|o| o.label)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            json_schema!({
                "type": "string",
                "enum": enum_values
            })
        }
        paperless_api_client::types::DataTypeEnum::Url
        | paperless_api_client::types::DataTypeEnum::Documentlink => {
            return None;
        }
    };
    // set the schema of the field value according to the type of custom field
    base_schema.get_mut("properties").map(|properties| {
        properties
            .get_mut("field_value")
            .map(|value_schema| *value_schema = field_schema.as_value().clone());
        properties.get_mut("alternative_values").map(|array| {
            array
                .get_mut("items")
                .map(|value_schema| *value_schema = field_schema.as_value().clone());
        });
        properties
    });
    Some(base_schema)
}
