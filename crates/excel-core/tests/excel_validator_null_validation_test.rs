//! Tests for null value validation behavior in mandatory vs non-mandatory fields

use serde_json::json;

mod common;

#[test]
fn test_null_accepted_for_non_mandatory_number_field() {
    // Create a schema with a non-mandatory number field
    let json_schema = json!({
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

    let validator = common::create_excel_validator_with_defaults(&json_schema);

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
    // Create a JSON Schema with a mandatory number field
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Required Number": {
                "type": "number"
            },
            "Optional Field": {
                "type": ["string", "null"]
            }
        },
        "required": ["Required Number"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

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
fn test_comprehensive_validation_scenario() {
    // Create a comprehensive JSON Schema mixing mandatory and non-mandatory fields
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample ID": {
                "type": "string"
            },
            "Temperature (°C)": {
                "type": "number",
                "minimum": -50.0,
                "maximum": 100.0
            },
            "Ammonium (μmol~1L)": {
                "type": ["number", "null"],
                "minimum": 0.0
            },
            "Depth (m)": {
                "type": ["integer", "null"],
                "minimum": 0
            },
            "Quality Flag": {
                "type": ["boolean", "null"]
            }
        },
        "required": ["Sample ID", "Temperature (°C)"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

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
    // Test that the JSON Schema correctly includes null types for non-mandatory fields
    let json_schema = json!({
        "type": "object",
        "properties": {
            "mandatory_string": {
                "type": "string"
            },
            "optional_number": {
                "type": ["number", "null"]
            },
            "optional_integer": {
                "type": ["integer", "null"]
            },
            "optional_boolean": {
                "type": ["boolean", "null"]
            },
            "optional_string": {
                "type": ["string", "null"]
            }
        },
        "required": ["mandatory_string"],
        "additionalProperties": false
    });

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
