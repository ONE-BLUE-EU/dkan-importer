//! Common test utilities for the dkan-importer library tests

use dkan_importer::model::ExcelValidator;
use serde_json::{json, Value};

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

/// Creates a test validator with the default test schema
#[allow(dead_code)]
pub fn create_test_validator() -> ExcelValidator {
    let schema = create_test_schema();
    ExcelValidator::new(&schema).unwrap()
}

// Creates a test validator with a custom schema
// It is used in tests that need a custom schema
#[allow(dead_code)]
pub fn create_test_validator_with_schema(schema: &Value) -> ExcelValidator {
    ExcelValidator::new(schema).unwrap()
}
