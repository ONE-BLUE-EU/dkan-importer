//! Common test utilities for the dkan-importer library tests

use dkan_importer::model::ExcelValidator;
use serde_json::{json, Value};
use std::collections::HashMap;

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
    let title_to_name_mapping = create_test_title_to_name_mapping();
    ExcelValidator::new(&schema, title_to_name_mapping).unwrap()
}

// Creates a test validator with a custom schema
// It is used in tests that need a custom schema
#[allow(dead_code)]
pub fn create_test_validator_with_schema(schema: &Value) -> ExcelValidator {
    let title_to_name_mapping = create_title_to_name_mapping_from_schema(schema);
    ExcelValidator::new(schema, title_to_name_mapping).unwrap()
}

// Creates a test title-to-name mapping for testing
// Maps the titles used in test schemas to their corresponding machine names
#[allow(dead_code)]
pub fn create_test_title_to_name_mapping() -> HashMap<String, String> {
    let mut mapping = HashMap::new();

    // Default test schema mappings (basic schema fields)
    mapping.insert("name".to_string(), "name".to_string());
    mapping.insert("age".to_string(), "age".to_string());
    mapping.insert("email".to_string(), "email".to_string());
    mapping.insert("active".to_string(), "active".to_string());
    mapping.insert("score".to_string(), "score".to_string());

    // Common test schema mappings used in other tests
    mapping.insert("Sample ID".to_string(), "sample_id".to_string());
    mapping.insert("Temperature".to_string(), "temperature".to_string());
    mapping.insert("Date".to_string(), "date".to_string());
    mapping.insert("Notes".to_string(), "notes".to_string());
    mapping.insert("Volume (mL) *".to_string(), "volume_ml".to_string());
    mapping.insert("Ammonium".to_string(), "ammonium".to_string());
    mapping.insert("Required ID".to_string(), "required_id".to_string());
    mapping.insert(
        "Required Category *".to_string(),
        "required_category".to_string(),
    );
    mapping.insert(
        "Optional Description".to_string(),
        "optional_description".to_string(),
    );
    mapping.insert(
        "Sample Identifier".to_string(),
        "sample_identifier".to_string(),
    );
    mapping.insert(
        "Collection Date *".to_string(),
        "collection_date".to_string(),
    );
    mapping.insert("Depth (m)".to_string(), "depth_m".to_string());
    mapping.insert("Temperature (°C)".to_string(), "temperature_c".to_string());
    mapping.insert("Salinity".to_string(), "salinity".to_string());

    mapping
}

// Creates a mapping from a schema by inferring names from the properties
// This is useful for tests that create custom schemas
#[allow(dead_code)]
pub fn create_title_to_name_mapping_from_schema(schema: &Value) -> HashMap<String, String> {
    let mut mapping = HashMap::new();

    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        for (property_name, _property_schema) in properties {
            // For test purposes, assume property names are the same as their machine names
            // In real scenarios, the mapping would be created from the data dictionary
            mapping.insert(property_name.clone(), property_name.clone());
        }
    }

    mapping
}
