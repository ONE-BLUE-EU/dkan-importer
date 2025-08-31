//! Tests for null value validation behavior in mandatory vs non-mandatory fields

use calamine::Data;
use dkan_importer::model::{data_dictionary::DataDictionary, ExcelValidator};
use serde_json::{json, Value};

mod common;

#[test]
fn test_null_accepted_for_non_mandatory_number_field() {
    // Create a schema with a non-mandatory number field
    let dkan_schema = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "required_field",
                "title": "Required Field",
                "type": "string",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "optional_number",
                "title": "Optional Number",
                "type": "number",
                "constraints": {
                    "required": false
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();

    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test data with null value for non-mandatory field (use title-based property names)
    let test_data = json!({
        "Required Field": "test value",
        "Optional Number": null
    });

    // Validate directly using the jsonschema validator
    let is_valid = validator.validator.is_valid(&test_data);
    if !is_valid {
        // Validation errors exist but are not printed in unit tests
        panic!("Validation should succeed for null value in non-mandatory field");
    }
}

#[test]
fn test_null_rejected_for_mandatory_number_field() {
    // Create a schema with a mandatory number field
    let dkan_schema = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "required_number",
                "title": "Required Number",
                "type": "number",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "optional_field",
                "title": "Optional Field",
                "type": "string",
                "constraints": {
                    "required": false
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();

    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test data with null value for mandatory field (use title-based property names)
    let test_data = json!({
        "Required Number": null,
        "Optional Field": "test"
    });

    // Validation should fail
    let is_valid = validator.validator.is_valid(&test_data);
    if is_valid {
        panic!("Validation should fail for null value in mandatory field");
    } else {
        // Expected validation errors exist
    }
}

#[test]
fn test_empty_cell_conversion_for_non_mandatory_number() {
    // Create a schema with non-mandatory number fields
    let dkan_schema = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "required_name",
                "title": "Required Name",
                "type": "string",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "optional_age",
                "title": "Optional Age",
                "type": "integer",
                "constraints": {
                    "required": false
                }
            },
            {
                "name": "optional_score",
                "title": "Optional Score",
                "type": "number",
                "constraints": {
                    "required": false
                }
            },
            {
                "name": "optional_active",
                "title": "Optional Active",
                "type": "boolean",
                "constraints": {
                    "required": false
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test empty cell conversion for different field types
    let empty_cell = Data::Empty;

    // Test for non-mandatory integer field (use title-based property name)
    let age_result =
        validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Age");
    assert_eq!(
        age_result,
        Value::Null,
        "Empty cell should convert to null for non-mandatory integer field"
    );

    // Test for non-mandatory number field
    let score_result =
        validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Score");
    assert_eq!(
        score_result,
        Value::Null,
        "Empty cell should convert to null for non-mandatory number field"
    );

    // Test for non-mandatory boolean field
    let active_result =
        validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Active");
    assert_eq!(
        active_result,
        Value::Null,
        "Empty cell should convert to null for non-mandatory boolean field"
    );
}

#[test]
fn test_empty_string_conversion_for_non_mandatory_number() {
    // Create a schema with non-mandatory number field
    let dkan_schema = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "required_name",
                "title": "Required Name",
                "type": "string",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "optional_amount",
                "title": "Optional Amount",
                "type": "number",
                "constraints": {
                    "required": false
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test empty string conversion for non-mandatory number field
    // Use title-based property name
    let empty_string_cell = Data::String("".to_string());
    let result =
        validator.convert_cell_to_json_with_schema_awareness(&empty_string_cell, "Optional Amount");
    assert_eq!(
        result,
        Value::Null,
        "Empty string should convert to null for non-mandatory number field"
    );

    // Test whitespace-only string
    let whitespace_cell = Data::String("   ".to_string());
    let result2 =
        validator.convert_cell_to_json_with_schema_awareness(&whitespace_cell, "Optional Amount");
    assert_eq!(
        result2,
        Value::Null,
        "Whitespace-only string should convert to null for non-mandatory number field"
    );
}

#[test]
fn test_comprehensive_validation_scenario() {
    // Create a comprehensive schema mixing mandatory and non-mandatory fields
    let dkan_schema = json!({
        "title": "Comprehensive Test Schema",
        "fields": [
            {
                "name": "sample_id",
                "title": "Sample ID",
                "type": "string",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "temperature",
                "title": "Temperature (°C)",
                "type": "number",
                "constraints": {
                    "required": true,
                    "minimum": -50.0,
                    "maximum": 100.0
                }
            },
            {
                "name": "ammonium",
                "title": "Ammonium (μmol~1L)",
                "type": "number",
                "constraints": {
                    "required": false,
                    "minimum": 0.0
                }
            },
            {
                "name": "depth",
                "title": "Depth (m)",
                "type": "integer",
                "constraints": {
                    "required": false,
                    "minimum": 0
                }
            },
            {
                "name": "quality_flag",
                "title": "Quality Flag",
                "type": "boolean",
                "constraints": {
                    "required": false
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test case 1: Valid data with some null optional fields (use title-based property names)
    let valid_data = json!({
        "Sample ID": "SAMPLE_001",
        "Temperature (°C)": 15.5,
        "Ammonium (μmol~1L)": null,      // Non-mandatory, should be allowed
        "Depth (m)": null,         // Non-mandatory, should be allowed
        "Quality Flag": true
    });

    let result1 = validator.validator.is_valid(&valid_data);
    assert!(
        result1,
        "Validation should succeed with null values in non-mandatory fields"
    );

    // Test case 2: Invalid data with null mandatory field
    let invalid_data = json!({
        "Sample ID": "SAMPLE_002",
        "Temperature (°C)": null,   // Mandatory, should be rejected
        "Ammonium (μmol~1L)": 5.2,
        "Depth (m)": 10,
        "Quality Flag": false
    });

    let result2 = validator.validator.is_valid(&invalid_data);
    assert!(
        !result2,
        "Validation should fail with null value in mandatory field"
    );

    // Test case 3: All optional fields null
    let all_optional_null = json!({
        "Sample ID": "SAMPLE_003",
        "Temperature (°C)": 22.0,
        "Ammonium (μmol~1L)": null,
        "Depth (m)": null,
        "Quality Flag": null
    });

    let result3 = validator.validator.is_valid(&all_optional_null);
    assert!(
        result3,
        "Validation should succeed with all optional fields as null"
    );
}

#[test]
fn test_schema_generation_includes_null_types() {
    // Test that the schema generation correctly includes null types for non-mandatory fields
    let dkan_schema = json!({
        "title": "Schema Generation Test",
        "fields": [
            {
                "name": "mandatory_string",
                "type": "string",
                "constraints": { "required": true }
            },
            {
                "name": "optional_number",
                "type": "number",
                "constraints": { "required": false }
            },
            {
                "name": "optional_integer",
                "type": "integer",
                "constraints": { "required": false }
            },
            {
                "name": "optional_boolean",
                "type": "boolean",
                "constraints": { "required": false }
            },
            {
                "name": "optional_string",
                "type": "string",
                "constraints": { "required": false }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();

    // Check mandatory string field - should be just "string"
    let mandatory_string = properties.get("mandatory_string").unwrap();
    assert_eq!(mandatory_string.get("type").unwrap(), &json!("string"));

    // Check optional number field - should be ["number", "null"]
    let optional_number = properties.get("optional_number").unwrap();
    let number_type = optional_number.get("type").unwrap();
    assert_eq!(
        number_type,
        &json!(["number", "null"]),
        "Optional number field should allow null"
    );

    // Check optional integer field - should be ["integer", "null"]
    let optional_integer = properties.get("optional_integer").unwrap();
    let integer_type = optional_integer.get("type").unwrap();
    assert_eq!(
        integer_type,
        &json!(["integer", "null"]),
        "Optional integer field should allow null"
    );

    // Check optional boolean field - should be ["boolean", "null"]
    let optional_boolean = properties.get("optional_boolean").unwrap();
    let boolean_type = optional_boolean.get("type").unwrap();
    assert_eq!(
        boolean_type,
        &json!(["boolean", "null"]),
        "Optional boolean field should allow null"
    );

    // Check optional string field - should now be ["string", "null"] union (new behavior)
    let optional_string = properties.get("optional_string").unwrap();
    assert_eq!(
        optional_string.get("type").unwrap(),
        &json!(["string", "null"])
    );

    // Check required fields list
    let required_fields = json_schema.get("required").unwrap().as_array().unwrap();
    assert_eq!(required_fields, &vec![json!("mandatory_string")]);
}
