//! Regression test for the specific issue mentioned by the user
//! This test simulates the exact scenario that was failing:
//! "Type mismatch at row[2]./Ammonium (μmol~1L): expected number, got null"

use dkan_importer::model::{data_dictionary::DataDictionary, ExcelValidator};
use serde_json::json;

mod common;

#[test]
fn test_ammonium_null_value_acceptance() {
    // Create a schema that mimics a real scientific data collection schema
    // with the specific "Ammonium (μmol~1L)" field that was causing the issue
    let dkan_schema = json!({
        "title": "Sample Collection Schema",
        "fields": [
            {
                "name": "Sample ID",
                "title": "Sample ID",
                "type": "string",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "Ammonium (μmol~1L)",
                "title": "Ammonium (μmol~1L)",
                "type": "number",
                "constraints": {
                    "required": false,  // This is key - it's not mandatory
                    "minimum": 0.0
                }
            },
            {
                "name": "Temperature (°C)",
                "title": "Temperature (°C)",
                "type": "number",
                "constraints": {
                    "required": true,
                    "minimum": -50.0,
                    "maximum": 100.0
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();

    let error_log_file = common::create_test_error_log_file();
    let validator =
        ExcelValidator::new(&json_schema, error_log_file.path().to_str().unwrap()).unwrap();

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

#[test]
fn test_excel_cell_to_null_conversion() {
    use calamine::Data;

    // Create the same schema as above
    let dkan_schema = json!({
        "title": "Sample Collection Schema",
        "fields": [
            {
                "name": "Sample ID",
                "title": "Sample ID",
                "type": "string",
                "constraints": { "required": true }
            },
            {
                "name": "Ammonium (μmol~1L)",
                "title": "Ammonium (μmol~1L)",
                "type": "number",
                "constraints": { "required": false, "minimum": 0.0 }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let error_log_file = common::create_test_error_log_file();
    let validator =
        ExcelValidator::new(&json_schema, error_log_file.path().to_str().unwrap()).unwrap();

    // Test empty Excel cell conversion for the Ammonium field
    let empty_cell = Data::Empty;
    let converted_value =
        validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Ammonium (μmol~1L)");

    assert_eq!(
        converted_value,
        serde_json::Value::Null,
        "Empty cell should be converted to null for non-mandatory Ammonium field"
    );

    // Test empty string conversion
    let empty_string_cell = Data::String("".to_string());
    let converted_empty_string = validator
        .convert_cell_to_json_with_schema_awareness(&empty_string_cell, "Ammonium (μmol~1L)");

    assert_eq!(
        converted_empty_string,
        serde_json::Value::Null,
        "Empty string should be converted to null for non-mandatory Ammonium field"
    );
}
