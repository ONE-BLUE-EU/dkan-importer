//! Regression test for the specific issue mentioned by the user
//! This test simulates the exact scenario that was failing:
//! "Type mismatch at row[2]./Ammonium (μmol~1L): expected number, got null"

use serde_json::json;

mod common;

#[test]
fn test_ammonium_null_value_acceptance() {
    // Create a JSON Schema that mimics a real scientific data collection schema
    // with the specific "Ammonium (μmol~1L)" field that was causing the issue
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample ID": {
                "type": "string"
            },
            "Ammonium (μmol~1L)": {
                "type": ["number", "null"],
                "minimum": 0.0
            },
            "Temperature (°C)": {
                "type": "number",
                "minimum": -50.0,
                "maximum": 100.0
            }
        },
        "required": ["Sample ID", "Temperature (°C)"],
        "additionalProperties": false
    });

    let validator = common::create_excel_validator_with_defaults(&json_schema);

    // Test the exact scenario that was failing before:
    // Row 2 with null Ammonium value (the original error message)
    let test_data_row2 = json!({
        "Sample ID": "SAMPLE_002",
        "Ammonium (μmol~1L)": null,  // This was causing the error
        "Temperature (°C)": 15.5
    });

    // This should now pass because Ammonium is not mandatory
    let is_valid = validator.validator.is_valid(&test_data_row2);
    if !is_valid {
        // Validation errors exist but are not printed
    }
    assert!(
        is_valid,
        "Row 2 with null Ammonium should be valid since Ammonium is not mandatory"
    );

    // Test another scenario: mandatory field is null (should fail)
    let test_data_invalid = json!({
        "Sample ID": "SAMPLE_003",
        "Ammonium (μmol~1L)": 5.2,
        "Temperature (°C)": null  // Temperature is mandatory, should fail
    });

    let is_valid_invalid = validator.validator.is_valid(&test_data_invalid);
    assert!(
        !is_valid_invalid,
        "Row with null mandatory Temperature should be invalid"
    );

    // Test valid data with actual Ammonium value
    let test_data_valid = json!({
        "Sample ID": "SAMPLE_004",
        "Ammonium (μmol~1L)": 3.2,
        "Temperature (°C)": 22.0
    });

    let is_valid_with_value = validator.validator.is_valid(&test_data_valid);
    assert!(
        is_valid_with_value,
        "Row with actual Ammonium value should be valid"
    );
}
