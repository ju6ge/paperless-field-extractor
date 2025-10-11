use chrono::NaiveDate;
use paperless_api_client::types::{CustomField, CustomFieldInstance, DataTypeEnum};
use schemars::{JsonSchema, json_schema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Serialize, Deserialize, JsonSchema)]
/// Structure to extract currency data from a document
pub(crate) struct CurrencyValue {
    value: f64,
    currency_code: String,
}

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

#[derive(Debug, Error)]
pub(crate) enum FieldError {
    #[error("custom fields with type {0} are not supported!")]
    UnsupportedCustomFieldType(DataTypeEnum),
    #[error("could not parse custom field value to the expected custom field type")]
    ParsingError(#[from] serde_json::error::Error),
    #[error("custom field `{0}` is of type enum but has no variants defined!")]
    EmptyEnumValuesForCustomField(String),
    #[error("matching {0} variant does not uniquely match any variant of custom field {1}")]
    NoUniqueEnumValueFound(String, String),
}

impl FieldExtract {
    pub fn to_custom_field_instance(
        &self,
        custom_field_spec: &CustomField,
    ) -> Result<CustomFieldInstance, FieldError> {
        // try to parse the value into the specified type and transform
        // it to the representation required for paperless to set
        // custom field based via the API
        match custom_field_spec.data_type {
            paperless_api_client::types::DataTypeEnum::Boolean => {
                let _parsed_value: bool = serde_json::from_value(self.field_value.clone())?;
                Ok(CustomFieldInstance {
                    value: Some(self.field_value.clone()),
                    field: custom_field_spec.id,
                })
            }
            paperless_api_client::types::DataTypeEnum::String => {
                let _parsed_value: String = serde_json::from_value(self.field_value.clone())?;
                Ok(CustomFieldInstance {
                    value: Some(self.field_value.clone()),
                    field: custom_field_spec.id,
                })
            }
            paperless_api_client::types::DataTypeEnum::Date => {
                let _parsed_value: NaiveDate = serde_json::from_value(self.field_value.clone())?;
                Ok(CustomFieldInstance {
                    value: Some(self.field_value.clone()),
                    field: custom_field_spec.id,
                })
            }
            paperless_api_client::types::DataTypeEnum::Integer => {
                let _parsed_value: i64 = serde_json::from_value(self.field_value.clone())?;
                Ok(CustomFieldInstance {
                    value: Some(self.field_value.clone()),
                    field: custom_field_spec.id,
                })
            }
            paperless_api_client::types::DataTypeEnum::Float => {
                let _parsed_value: f64 = serde_json::from_value(self.field_value.clone())?;
                Ok(CustomFieldInstance {
                    value: Some(self.field_value.clone()),
                    field: custom_field_spec.id,
                })
            }
            paperless_api_client::types::DataTypeEnum::Monetary => {
                let parsed_currency_value: CurrencyValue =
                    serde_json::from_value(self.field_value.clone())?;
                Ok(CustomFieldInstance {
                    value: Some(Value::String(format!(
                        "{}{}",
                        parsed_currency_value.currency_code, parsed_currency_value.value
                    ))),
                    field: custom_field_spec.id,
                })
            }
            paperless_api_client::types::DataTypeEnum::Select => {
                let enum_variant: String = serde_json::from_value(self.field_value.clone())?;
                let select_options: FieldSelect = if let Some(v) = &custom_field_spec.extra_data {
                    serde_json::from_value(v.clone()).unwrap()
                } else {
                    return Err(FieldError::EmptyEnumValuesForCustomField(
                        custom_field_spec.name.clone(),
                    ));
                };
                let matching_enum_values = &select_options
                    .select_options
                    .into_iter()
                    .filter(|cfi| *cfi.label == enum_variant)
                    .collect::<Vec<_>>();
                if matching_enum_values.len() != 1 {
                    Err(FieldError::NoUniqueEnumValueFound(
                        enum_variant,
                        custom_field_spec.name.clone(),
                    ))
                } else {
                    let matching_enum_variant = matching_enum_values
                        .first()
                        .expect("if case checks that exactly one element exists");
                    Ok(CustomFieldInstance {
                        // the value of a custom field corresponds to the string id of the the enum variant field
                        value: Some(serde_json::Value::String(matching_enum_variant.id.clone())),
                        field: custom_field_spec.id,
                    })
                }
            }
            paperless_api_client::types::DataTypeEnum::Url
            | paperless_api_client::types::DataTypeEnum::Documentlink => Err(
                FieldError::UnsupportedCustomFieldType(custom_field_spec.data_type.clone()),
            ),
        }
        //TODO figure out what to do with alternative value suggestions
    }
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
        paperless_api_client::types::DataTypeEnum::Monetary => schema_for!(CurrencyValue),
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
