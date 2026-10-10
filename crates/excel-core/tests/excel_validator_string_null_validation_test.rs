//! Tests for null value validation in string fields
//! Non-mandatory string fields should now accept null values

use serde_json::json;

mod common;

#[test]
fn test_string_null_validation_behavior() {
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Required Name*": {
                "type": "string"
            },
            "Optional Description": {
                "type": ["string", "null"]
            },
            "Optional Notes": {
                "type": ["string", "null"]
            }
        },
        "required": ["Required Name*"],
        "additionalProperties": false
    });
    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // Test 1: Valid data with null optional string fields
    let valid_data = json!({
        "Required Name*": "John Doe",
        "Optional Description": null,  // This should now be accepted
        "Optional Notes": null         // This should now be accepted
    });

    let is_valid = validator.validator.is_valid(&valid_data);
    if !is_valid {
        // Validation errors exist but are not printed
    }
    assert!(is_valid, "Optional string fields should accept null values");

    // Test 2: Valid data with actual string values
    let valid_data2 = json!({
        "Required Name*": "Jane Smith",
        "Optional Description": "A detailed description",
        "Optional Notes": "Some notes here"
    });

    assert!(
        validator.validator.is_valid(&valid_data2),
        "String fields should accept string values"
    );

    // Test 3: Valid data with mixed null and string values
    let valid_data3 = json!({
        "Required Name*": "Bob Wilson",
        "Optional Description": "Has description",
        "Optional Notes": null  // One null, one string
    });

    assert!(
        validator.validator.is_valid(&valid_data3),
        "Mixed null and string values should work"
    );

    // Test 4: Invalid data - required string field is null
    let invalid_data = json!({
        "Required Name*": null,  // Required field cannot be null
        "Optional Description": "Some description",
        "Optional Notes": "Some notes"
    });

    assert!(
        !validator.validator.is_valid(&invalid_data),
        "Required string field should not accept null"
    );

    // Test 5: Invalid data - required string field missing
    let invalid_data2 = json!({
        // "Required Name *": missing
        "Optional Description": "Some description",
        "Optional Notes": null
    });

    assert!(
        !validator.validator.is_valid(&invalid_data2),
        "Missing required string field should fail"
    );
}

#[test]
fn test_regression_filter_cutoff_scenario() {
    // Test the specific scenario from the error log: "Filter cutoff (threshold)"
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample ID*": {
                "type": "string"
            },
            "Filter cutoff (threshold)": {
                "type": ["string", "null"]
            },
            "Measurement Value*": {
                "type": "number"
            }
        },
        "required": ["Sample ID*", "Measurement Value*"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // This scenario was failing before: optional string field with null value
    let test_data = json!({
        "Sample ID*": "SAMPLE_002",
        "Filter cutoff (threshold)": null,  // This was causing the error before
        "Measurement Value*": 42.5
    });

    let is_valid = validator.validator.is_valid(&test_data);
    if !is_valid {
        // Validation errors exist but are not printed
    }

    assert!(
        is_valid,
        "Optional string field should accept null - regression test for 'Filter cutoff (threshold)' error"
    );

    // Verify the schema structure
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();
    assert_eq!(
        properties
            .get("Filter cutoff (threshold)")
            .unwrap()
            .get("type")
            .unwrap(),
        &json!(["string", "null"])
    );
}
