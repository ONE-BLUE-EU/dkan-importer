use reqwest::blocking::Client;
use serde_json::{json, Value};

pub struct DataDictionary {
    pub id: String,
    pub name: String,
    pub fields: Value,
    pub url: String,
}

impl DataDictionary {
    pub fn new(
        base_url: &str,
        data_dictionary_id: &str,
        client: &Client,
    ) -> Result<Self, anyhow::Error> {
        let url = format!("{base_url}/api/1/metastore/schemas/data-dictionary/items");
        let response = client
            .get(&url)
            .header("Accept", "application/json")
            .header("Authorization", "Bearer <token>")
            .send()?;
        let body = response.text()?;

        // Parse the response as an array of schema objects
        let schemas: Vec<Value> = serde_json::from_str(&body)?;

        // Find the schema with matching title
        let matching_schema = schemas
            .into_iter()
            .find(|schema| {
                schema
                    .get("identifier")
                    .and_then(|identifier| identifier.as_str())
                    == Some(data_dictionary_id)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Data dictionary with identifier '{}' not found",
                    data_dictionary_id
                )
            })?;

        // Extract the data portion and convert to JSON Schema format
        let data = matching_schema
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("Data dictionary data not found"))?;

        let data_dictionary_url = format!(
            "{base_url}/api/1/metastore/schemas/data-dictionary/items/{data_dictionary_id}"
        );
        // Todo: Validate the URL is correct.
        let result = client.get(&data_dictionary_url).send()?;
        if !result.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to validate the existence of the data dictionary {data_dictionary_id}. \
                Please check if the data dictionary exists and is accessible at {data_dictionary_url}"
            ));
        }

        return Ok(DataDictionary {
            id: matching_schema
                .get("identifier")
                .and_then(|identifier| identifier.as_str())
                .expect("Data dictionary identifier not found")
                .to_string(),
            name: data
                .get("title")
                .and_then(|name| name.as_str())
                .expect("Data dictionary title not found")
                .to_string(),
            fields: data.clone(),
            url: data_dictionary_url,
        });
    }

    pub fn to_json_schema(&self) -> Result<Value, anyhow::Error> {
        Self::convert_data_dictionary_to_json_schema(&self.fields)
    }

    /// Convert DKAN data dictionary fields to JSON Schema format
    /// This is a static method that can be easily unit tested
    pub fn convert_data_dictionary_to_json_schema(
        dkan_fields: &Value,
    ) -> Result<Value, anyhow::Error> {
        let title = dkan_fields
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Untitled Schema");

        let fields = dkan_fields
            .get("fields")
            .and_then(|f| f.as_array())
            .ok_or_else(|| anyhow::anyhow!("Fields array not found in schema"))?;

        let mut properties = serde_json::Map::new();
        let mut required_fields = Vec::new();

        for field in fields {
            let field_name = field
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| anyhow::anyhow!("Field name not found"))?;
            let field_title = field.get("title").and_then(|t| t.as_str());
            let field_type = field
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string");
            let field_format = field.get("format").and_then(|f| f.as_str());
            let field_description = field.get("description").and_then(|d| d.as_str());

            // Check if either name or title ends with asterisk (*) - preserve original values
            let name_indicates_required = field_name.trim_end().ends_with('*');
            let title_indicates_required = if let Some(title) = field_title {
                title.trim_end().ends_with('*')
            } else {
                false
            };
            let asterisk_indicates_required = name_indicates_required || title_indicates_required;

            // Build JSON Schema property
            let mut property = serde_json::Map::new();

            // Map DKAN types to JSON Schema types
            let json_schema_type = match field_type {
                "integer" => "integer",
                "number" | "float" => "number",
                "boolean" => "boolean",
                "array" => "array",
                "object" => "object",
                "datetime" => "string", // treat datetime as string in JSON Schema
                _ => "string",
            };

            // Check if field will be required (check constraints and asterisk in name/title)
            let mut will_be_required = asterisk_indicates_required; // Start with asterisk indication
            if let Some(constraints) = field.get("constraints") {
                if let Some(required) = constraints.get("required") {
                    // Explicit constraints combine with asterisk indication
                    will_be_required = will_be_required || required.as_bool().unwrap_or(false);
                }
            }

            // For non-mandatory fields, allow null values by using union types
            if !will_be_required && !matches!(json_schema_type, "array" | "object") {
                // Allow null for number, integer, boolean, string fields when not mandatory
                property.insert("type".to_string(), json!([json_schema_type, "null"]));
            } else {
                property.insert("type".to_string(), json!(json_schema_type));
            }

            if let Some(title) = field_title {
                property.insert("title".to_string(), json!(title));
            }

            if let Some(description) = field_description {
                property.insert("description".to_string(), json!(description));
            }

            // Special handling for datetime
            if field_type == "datetime" {
                if let Some(format) = field_format {
                    if format != "default" && !format.is_empty() {
                        property.insert("format".to_string(), json!(format));
                        property.insert("dkan_format".to_string(), json!(format));
                    } else {
                        property.insert("format".to_string(), json!("date-time"));
                    }
                } else {
                    property.insert("format".to_string(), json!("date-time"));
                }
            } else if let Some(format) = field_format {
                if format != "default" && !format.is_empty() {
                    property.insert("format".to_string(), json!(format));
                }
            }

            // Add field to required list if it's marked as required (either by constraints or asterisk in title)
            if will_be_required {
                required_fields.push(field_name.to_string());
            }

            // Add any additional constraints based on field properties
            if let Some(constraints) = field.get("constraints") {
                // Note: required constraint is already handled above

                if let Some(min_length) = constraints.get("minLength") {
                    if let Some(min_len) = min_length.as_u64() {
                        property.insert("minLength".to_string(), json!(min_len));
                    }
                }

                if let Some(max_length) = constraints.get("maxLength") {
                    if let Some(max_len) = max_length.as_u64() {
                        property.insert("maxLength".to_string(), json!(max_len));
                    }
                }

                if let Some(minimum) = constraints.get("minimum") {
                    if let Some(min) = minimum.as_f64() {
                        property.insert("minimum".to_string(), json!(min));
                    }
                }

                if let Some(maximum) = constraints.get("maximum") {
                    if let Some(max) = maximum.as_f64() {
                        property.insert("maximum".to_string(), json!(max));
                    }
                }

                if let Some(pattern) = constraints.get("pattern") {
                    if let Some(pat) = pattern.as_str() {
                        property.insert("pattern".to_string(), json!(pat));
                    }
                }

                if let Some(enum_values) = constraints.get("enum") {
                    property.insert("enum".to_string(), enum_values.clone());
                }
            }

            properties.insert(field_name.to_string(), Value::Object(property));
        }

        // Build the complete JSON Schema
        let mut json_schema = serde_json::Map::new();
        json_schema.insert(
            "$schema".to_string(),
            json!("http://json-schema.org/draft-07/schema#"),
        );
        json_schema.insert("type".to_string(), json!("object"));
        json_schema.insert("title".to_string(), json!(title));
        json_schema.insert("properties".to_string(), Value::Object(properties));

        if !required_fields.is_empty() {
            json_schema.insert("required".to_string(), json!(required_fields));
        }

        // Add additionalProperties: false for strict validation
        json_schema.insert("additionalProperties".to_string(), json!(false));

        return Ok(Value::Object(json_schema));
    }
}
