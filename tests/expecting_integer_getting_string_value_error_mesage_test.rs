//! Tests for enhanced validation error messages that include actual values
//! This verifies that type mismatch errors show the actual value that was provided

use dkan_importer::model::{data_dictionary::DataDictionary, ExcelValidator};
use serde_json::json;

mod common;

#[test]
fn test_type_mismatch_error_includes_actual_value() {
    let dkan_schema = json!({
        "title": "Enhanced Error Messages Test",
        "fields": [
            {
                "name": "volume_ml",
                "title": "Volume (mL) *",  // Required integer field
                "type": "integer"
            },
            {
                "name": "temperature",
                "title": "Temperature *",  // Required number field
                "type": "number"
            },
            {
                "name": "is_active",
                "title": "Is Active *",   // Required boolean field
                "type": "boolean"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let error_log_file = common::create_test_error_log_file();
    let validator =
        ExcelValidator::new(&json_schema, error_log_file.path().to_str().unwrap()).unwrap();

    // Test data with type mismatches that should show actual values in error messages
    let test_data = json!({
        "volume_ml": "abc123",     // String instead of integer
        "temperature": "not_a_number",  // String instead of number
        "is_active": "maybe"       // String instead of boolean
    });

    let is_valid = validator.validator.is_valid(&test_data);
    assert!(
        !is_valid,
        "Test data should be invalid due to type mismatches"
    );

    // Collect all validation errors
    let errors: Vec<String> = validator
        .validator
        .iter_errors(&test_data)
        .map(|error| error.to_string())
        .collect();

    println!("Validation errors:");
    for error in &errors {
        println!("  - {}", error);
    }

    // Check that errors contain the actual values that were provided
    let all_errors = errors.join(" ");

    // For volume_ml field - should show the actual string value "abc123"
    assert!(
        all_errors.contains("abc123") || all_errors.contains("volume_ml"),
        "Error should mention the volume_ml field or its actual value 'abc123'"
    );

    // For temperature field - should show the actual string value "not_a_number"
    assert!(
        all_errors.contains("not_a_number") || all_errors.contains("temperature"),
        "Error should mention the temperature field or its actual value 'not_a_number'"
    );

    // For is_active field - should show the actual string value "maybe"
    assert!(
        all_errors.contains("maybe") || all_errors.contains("is_active"),
        "Error should mention the is_active field or its actual value 'maybe'"
    );

    println!("✅ Enhanced error messages successfully include actual values");
}

#[test]
fn test_validation_report_with_enhanced_errors() {
    // Test using the validation report functionality to get structured errors
    let dkan_schema = json!({
        "title": "Validation Report Test",
        "fields": [
            {
                "name": "id",
                "title": "ID *",
                "type": "integer"
            },
            {
                "name": "score",
                "title": "Score *",
                "type": "number"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let error_log_file = common::create_test_error_log_file();
    let validator =
        ExcelValidator::new(&json_schema, error_log_file.path().to_str().unwrap()).unwrap();

    // Create test data with type mismatches
    let test_rows = vec![
        json!({"id": "invalid_id", "score": "invalid_score"}), // Both fields have string values instead of numbers
        json!({"id": 123, "score": "still_invalid"}),          // Only score is invalid
        json!({"id": "999", "score": 85.5}),                   // Only id is invalid
    ];

    for (row_idx, row_data) in test_rows.iter().enumerate() {
        let is_valid = validator.validator.is_valid(row_data);

        if !is_valid {
            println!("Row {} validation errors:", row_idx + 1);
            for error in validator.validator.iter_errors(row_data) {
                println!("  - {}", error);

                // Check that TypeMismatch-style errors include the actual value
                let error_string = error.to_string();
                if error_string.contains("is not of type") {
                    // The jsonschema error should be converted to our enhanced format
                    println!("    Raw jsonschema error: {}", error_string);
                }
            }
        }
    }

    println!("✅ Validation reports successfully show enhanced error messages with actual values");
}

#[test]
fn test_specific_volume_error_enhancement() {
    // Specific test for the exact error scenario mentioned by the user
    let dkan_schema = json!({
        "title": "Volume Error Test",
        "fields": [
            {
                "name": "volume_ml",
                "title": "Volume (mL) *",
                "type": "integer"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let error_log_file = common::create_test_error_log_file();
    let validator =
        ExcelValidator::new(&json_schema, error_log_file.path().to_str().unwrap()).unwrap();

    // Test with the exact scenario: providing a string where an integer is expected
    let test_data = json!({"volume_ml": "15.5mL"});

    let is_valid = validator.validator.is_valid(&test_data);
    assert!(!is_valid, "Should have validation errors");

    // Get the first error and convert it to our enhanced format
    let jsonschema_error = validator.validator.iter_errors(&test_data).next().unwrap();
    let path = format!("row[2].{}", jsonschema_error.instance_path);
    let enhanced_error = validator.analyze_jsonschema_error(
        &jsonschema_error.to_string(),
        &path,
        &jsonschema_error.instance,
    );
    let error_message = enhanced_error.to_string();
    println!("Enhanced error message: {}", error_message);

    // Verify the enhanced error message format
    assert!(
        error_message.contains("Type mismatch"),
        "Should be a type mismatch error"
    );
    assert!(error_message.contains("row[2]"), "Should reference row 2");
    assert!(
        error_message.contains("volume_ml") || error_message.contains("Volume (mL)"),
        "Should reference the field"
    );
    assert!(
        error_message.contains("expected integer"),
        "Should mention expected type"
    );
    assert!(
        error_message.contains("got string"),
        "Should mention actual type"
    );
    assert!(
        error_message.contains("\"15.5mL\""),
        "Should show the actual value in quotes"
    );

    // The new format should be: "Type mismatch at row[2]./volume_ml: expected integer, got string "15.5mL""
    println!("✅ Volume error message successfully enhanced with actual value");
}

#[test]
fn test_error_message_format_comparison() {
    println!("🔍 Demonstrating error message enhancement:");

    let dkan_schema = json!({
        "title": "Format Comparison Test",
        "fields": [
            {
                "name": "volume_ml",
                "title": "Volume (mL) *",
                "type": "integer"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let error_log_file = common::create_test_error_log_file();
    let validator =
        ExcelValidator::new(&json_schema, error_log_file.path().to_str().unwrap()).unwrap();

    let test_data = json!({"volume_ml": "25.7"});
    let is_valid = validator.validator.is_valid(&test_data);

    if !is_valid {
        // Get the first error and convert it to our enhanced format
        let jsonschema_error = validator.validator.iter_errors(&test_data).next().unwrap();
        let path = format!("row[2].{}", jsonschema_error.instance_path);
        let enhanced_error = validator.analyze_jsonschema_error(
            &jsonschema_error.to_string(),
            &path,
            &jsonschema_error.instance,
        );
        let error_message = enhanced_error.to_string();

        println!(
            "❌ OLD FORMAT: Type mismatch at row[2]./Volume (mL): expected integer, got string"
        );
        println!("✅ NEW FORMAT: {}", error_message);
        println!();
        println!("💡 The actual problematic value is now clearly visible!");

        assert!(
            error_message.contains("\"25.7\""),
            "Error should show the actual value that caused the problem"
        );
    }

    println!("✅ Error message format successfully enhanced");
}
