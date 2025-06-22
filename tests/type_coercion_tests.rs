//! Tests for intelligent type coercion functionality

use calamine::Data;
use serde_json::{json, Value};

mod common;

#[test]
fn test_intelligent_type_coercion_string_to_integer() {
    let validator = common::create_test_validator();

    // Test string that should be converted to integer for "age" field
    let cell = Data::String("25".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "age");

    assert_eq!(result, json!(25));
}

#[test]
fn test_intelligent_type_coercion_string_to_boolean() {
    let validator = common::create_test_validator();

    // Test string that should be converted to boolean for "active" field
    let cell = Data::String("true".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "active");

    assert_eq!(result, json!(true));

    let cell2 = Data::String("yes".to_string());
    let result2 = validator.convert_cell_to_json_with_schema_awareness(&cell2, "active");

    assert_eq!(result2, json!(true));
}

#[test]
fn test_fallback_intelligent_conversion() {
    let validator = common::create_test_validator();

    // Test conversion for unknown field (should fall back to intelligent conversion)
    let cell = Data::String("42".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "unknown_field");

    assert_eq!(result, json!(42));
}

#[test]
fn test_bounds_checking_in_coercion() {
    let bounded_schema = json!({
        "type": "object",
        "properties": {
            "limited_number": {
                "type": "integer",
                "minimum": 1,
                "maximum": 10
            }
        }
    });

    let validator = common::create_test_validator_with_schema(&bounded_schema);

    // Test value within bounds
    let cell1 = Data::String("5".to_string());
    let result1 = validator.convert_cell_to_json_with_schema_awareness(&cell1, "limited_number");
    assert_eq!(result1, json!(5));

    // Test value outside bounds (should remain as string)
    let cell2 = Data::String("15".to_string());
    let result2 = validator.convert_cell_to_json_with_schema_awareness(&cell2, "limited_number");
    assert_eq!(result2, Value::String("15".to_string()));
}

#[test]
fn test_enum_case_insensitive_matching() {
    let enum_schema = json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["Active", "Inactive", "Pending"]
            }
        }
    });

    let validator = common::create_test_validator_with_schema(&enum_schema);

    // Test case-insensitive enum matching
    let cell = Data::String("active".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "status");
    assert_eq!(result, Value::String("Active".to_string()));
}

#[test]
fn test_array_delimiter_splitting() {
    let array_schema = json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "array",
                "items": {"type": "string"}
            }
        }
    });

    let validator = common::create_test_validator_with_schema(&array_schema);

    // Test comma-separated values
    let cell = Data::String("tag1,tag2,tag3".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "tags");
    assert_eq!(
        result,
        Value::Array(vec![
            Value::String("tag1".to_string()),
            Value::String("tag2".to_string()),
            Value::String("tag3".to_string())
        ])
    );
}

#[test]
fn test_mixed_type_schema() {
    let mixed_schema = json!({
        "type": "object",
        "properties": {
            "flexible_field": {
                "type": ["string", "number", "boolean"]
            }
        }
    });

    let validator = common::create_test_validator_with_schema(&mixed_schema);

    // Test that it tries number first for numeric strings
    let cell = Data::String("42".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "flexible_field");
    assert_eq!(result, json!(42));
}
