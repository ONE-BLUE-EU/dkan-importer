// Test utilities available to both unit and integration tests
// Only compiled when testing

use crate::excel_validator::ExcelValidator;
use crate::utils::normalize_string;
use serde_json::{Value, json};

/// [LEGACY] Helper function to normalize DKAN field data for testing
#[allow(dead_code)]
pub fn normalize_field_data_for_tests(json_schema: Value) -> Result<Value, anyhow::Error> {
    let mut normalized = json_schema.clone();

    if let Some(fields) = normalized.get_mut("fields").and_then(|f| f.as_array_mut()) {
        for field in fields.iter_mut() {
            if let Some(field_obj) = field.as_object_mut() {
                // Normalize title if present
                if let Some(title) = field_obj.get("title").and_then(|t| t.as_str()) {
                    field_obj.insert("title".to_string(), json!(normalize_string(title)));
                }
            }
        }
    }

    Ok(normalized)
}

/// Factory function with default test file paths
#[allow(dead_code)]
pub fn create_excel_validator_with_defaults(schema: &Value) -> ExcelValidator {
    ExcelValidator::new_for_testing(schema).unwrap()
}

/// Creates a basic test schema for testing purposes
#[allow(dead_code)]
pub fn create_test_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "minLength": 1
            },
            "age": {
                "type": "integer",
                "minimum": 0,
                "maximum": 150
            },
            "email": {
                "type": "string",
                "format": "email"
            },
            "active": {
                "type": "boolean"
            },
            "score": {
                "type": "number",
                "minimum": 0.0,
                "maximum": 100.0
            }
        },
        "required": ["name", "age", "email"],
        "additionalProperties": false
    })
}
