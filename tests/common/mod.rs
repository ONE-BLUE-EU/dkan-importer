//! Common test utilities for the dkan-importer library tests

use dkan_importer::model::{DataDictionary, ExcelValidator};
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

/// Converts JSON Schema format to DKAN data dictionary format for testing
/// This allows us to use the production DataDictionary::create_title_to_name_mapping method
/// with test schemas that are in JSON Schema format
#[allow(dead_code)]
fn convert_schema_to_dkan_format(schema: &Value) -> Value {
    let mut dkan_format = serde_json::Map::new();
    
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        let mut fields = Vec::new();
        
        for (property_name, property_schema) in properties {
            let mut field = serde_json::Map::new();
            field.insert("name".to_string(), json!(property_name));
            field.insert("title".to_string(), json!(property_name));
            
            // Copy over the type and other schema properties
            if let Some(field_type) = property_schema.get("type") {
                field.insert("type".to_string(), field_type.clone());
            }
            
            // Copy constraints if they exist
            if let Some(constraints) = property_schema.get("minimum") {
                field.insert("minimum".to_string(), constraints.clone());
            }
            if let Some(constraints) = property_schema.get("maximum") {
                field.insert("maximum".to_string(), constraints.clone());
            }
            if let Some(constraints) = property_schema.get("minLength") {
                field.insert("minLength".to_string(), constraints.clone());
            }
            if let Some(constraints) = property_schema.get("maxLength") {
                field.insert("maxLength".to_string(), constraints.clone());
            }
            if let Some(constraints) = property_schema.get("format") {
                field.insert("format".to_string(), constraints.clone());
            }
            if let Some(constraints) = property_schema.get("pattern") {
                field.insert("pattern".to_string(), constraints.clone());
            }
            if let Some(constraints) = property_schema.get("enum") {
                field.insert("enum".to_string(), constraints.clone());
            }
            
            fields.push(Value::Object(field));
        }
        
        dkan_format.insert("fields".to_string(), json!(fields));
    }
    
    // Add title if it exists in the original schema
    if let Some(title) = schema.get("title") {
        dkan_format.insert("title".to_string(), title.clone());
    }
    
    Value::Object(dkan_format)
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
    // Convert JSON Schema format to DKAN format for the production method
    let dkan_format = convert_schema_to_dkan_format(schema);
    let title_to_name_mapping = DataDictionary::create_title_to_name_mapping(&dkan_format).unwrap();
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
