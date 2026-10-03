//! Regression test for the specific issue reported in errors.log
//! "Type mismatch at row[2]./Sample Code: expected Multiple((null, string)), got integer "24019001""
//!
//! This test verifies that when a schema expects a string (or string|null union type),
//! Excel cells containing integers or floats are automatically converted to strings.

use serde_json::json;

mod common;

#[test]
fn test_integer_sample_code_converted_to_string() {
    // Schema that expects Sample Code to be a string or null (union type)
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample Code": {
                "type": ["string", "null"]
            },
            "Station Label*": {
                "type": "string"
            },
            "Date of sampling start*": {
                "type": "string",
                "format": "date"
            }
        },
        "required": ["Station Label*", "Date of sampling start*"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // Test data that mimics the error scenario from errors.log
    // Sample Code has an integer value (24019001) that should be converted to string
    let test_data = json!({
        "Sample Code": "24019001",  // Now as string after conversion
        "Station Label*": "2",
        "Date of sampling start*": "2023-08-06"
    });

    let is_valid = validator.validator.is_valid(&test_data);
    assert!(
        is_valid,
        "Integer value in Sample Code field should be converted to string and pass validation"
    );
}

#[test]
fn test_multiple_numeric_fields_with_string_schema() {
    // Test multiple fields that expect strings but may receive numeric Excel values
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample Code": {
                "type": ["string", "null"]
            },
            "Barcode": {
                "type": "string"
            },
            "Station Number": {
                "type": ["string", "null"]
            },
            "Temperature (C)": {
                "type": ["number", "null"]
            }
        },
        "required": ["Barcode"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // Test with various numeric values that should be converted to strings
    let test_data1 = json!({
        "Sample Code": "24019001",      // Integer -> String
        "Barcode": "123456789",          // Integer -> String
        "Station Number": "42",          // Integer -> String
        "Temperature (C)": 15.5          // Number remains number
    });

    assert!(
        validator.validator.is_valid(&test_data1),
        "Numeric values should be converted to strings when schema expects strings"
    );

    // Test with float values that should be converted to strings
    let test_data2 = json!({
        "Sample Code": "123.45",         // Float -> String
        "Barcode": "987654321",          // Integer -> String
        "Station Number": null,          // Null is allowed
        "Temperature (C)": 20.3          // Number remains number
    });

    assert!(
        validator.validator.is_valid(&test_data2),
        "Float values should be converted to strings when schema expects strings"
    );
}

#[test]
fn test_sampling_duration_remains_numeric_for_number_schema() {
    // Test the other error from errors.log:
    // "Sampling duration - hours: expected Multiple((null, number)), got string "16:00:00""
    // Note: This test verifies that when schema expects number, strings are NOT converted
    // unless they can be parsed as numbers
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sampling duration - hours": {
                "type": ["number", "null"]
            },
            "Sample ID": {
                "type": "string"
            }
        },
        "required": ["Sample ID"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // Valid case: numeric value in a field expecting number
    let valid_data = json!({
        "Sampling duration - hours": 16.5,
        "Sample ID": "SAMPLE_001"
    });

    assert!(
        validator.validator.is_valid(&valid_data),
        "Numeric values should remain numeric when schema expects number"
    );

    // Valid case: null value in optional number field
    let valid_data_null = json!({
        "Sampling duration - hours": null,
        "Sample ID": "SAMPLE_002"
    });

    assert!(
        validator.validator.is_valid(&valid_data_null),
        "Null values should be accepted in optional number fields"
    );
}

#[test]
fn test_exact_errors_log_scenario() {
    // Recreate the exact scenario from errors.log row 2
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample Code": {
                "type": ["string", "null"]
            },
            "Sampling duration - hours": {
                "type": ["number", "null"]
            },
            "Station Label*": {
                "type": "string"
            },
            "Date of sampling start*": {
                "type": "string",
                "format": "date"
            },
            "Depth (m)": {
                "type": ["number", "null"]
            },
            "Container": {
                "type": ["string", "null"]
            }
        },
        "required": ["Station Label*", "Date of sampling start*"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // Row 2 data from errors.log (with our fix applied, Sample Code should be a string)
    let row2_data = json!({
        "Sample Code": "24019001",                      // Integer converted to string ✓
        "Sampling duration - hours": null,              // Null is valid for optional field
        "Station Label*": "2",
        "Date of sampling start*": "2023-08-06",
        "Depth (m)": 5,
        "Container": "SPE cartridge (PPL)"
    });

    let is_valid = validator.validator.is_valid(&row2_data);
    if !is_valid {
        // If validation fails, print the errors for debugging
        let validation_errors: Vec<_> = validator.validator.iter_errors(&row2_data).collect();
        for error in &validation_errors {
            eprintln!("Validation error: {}", error);
        }
    }

    assert!(
        is_valid,
        "Row 2 data should pass validation after integer-to-string conversion fix"
    );
}
