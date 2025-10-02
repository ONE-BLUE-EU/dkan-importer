use importer_lib::ExcelValidator;
use serde_json::{Value, json};
use std::collections::HashMap;

// Re-export shared test utilities from src/test_utils.rs
// These are the core functions used by most tests
pub use importer_lib::test_utils::{create_excel_validator_with_defaults, create_test_schema};

// =============================================================================
// Modern JSON Schema Helpers - Use these for new tests
// =============================================================================

/// Create a simple JSON Schema with the given properties
#[allow(dead_code)]
pub fn create_schema(properties: Value, required: Vec<&str>) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

/// Create a simple title-to-name mapping from property names
#[allow(dead_code)]
pub fn create_simple_mapping(property_names: Vec<&str>) -> HashMap<String, String> {
    property_names
        .into_iter()
        .map(|name| (name.to_string(), name.to_string()))
        .collect()
}

/// Create a property schema with optional null union
#[allow(dead_code)]
pub fn create_property(base_type: &str, required: bool) -> Value {
    if required || matches!(base_type, "array" | "object") {
        json!({ "type": base_type })
    } else {
        json!({ "type": [base_type, "null"] })
    }
}

// =============================================================================
// Additional helpers for integration tests
// =============================================================================

/// Create title-to-name mapping from DKAN schema
#[allow(dead_code)]
pub fn create_title_to_name_mapping(
    json_schema: &Value,
) -> Result<HashMap<String, String>, anyhow::Error> {
    use importer_lib::utils::normalize_string;
    let mut mapping = HashMap::new();

    if let Some(fields) = json_schema.get("fields").and_then(|f| f.as_array()) {
        for field in fields {
            if let Some(field_obj) = field.as_object() {
                let name = field_obj
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let title = field_obj
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or(&name)
                    .to_string();

                mapping.insert(normalize_string(&title), name);
            }
        }
    }

    Ok(mapping)
}

/// Creates a test validator with a custom schema
#[allow(dead_code)]
pub fn create_test_validator_with_schema(schema: &Value) -> ExcelValidator {
    create_excel_validator_with_defaults(schema)
}

/// Creates a test validator with the default test schema
#[allow(dead_code)]
pub fn create_test_validator() -> ExcelValidator {
    let schema = create_test_schema();
    create_excel_validator_with_defaults(&schema)
}

/// Create default test title-to-name mapping
#[allow(dead_code)]
pub fn create_test_title_to_name_mapping() -> HashMap<String, String> {
    let mut mapping = HashMap::new();

    // Default test schema mappings
    mapping.insert("name".to_string(), "name".to_string());
    mapping.insert("age".to_string(), "age".to_string());
    mapping.insert("email".to_string(), "email".to_string());
    mapping.insert("active".to_string(), "active".to_string());
    mapping.insert("score".to_string(), "score".to_string());

    // Common test mappings
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
