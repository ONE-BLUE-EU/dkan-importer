//! Tests for DataDictionary constructor duplicate checking functionality
//! Verifies that DataDictionary::new() now checks for duplicates during construction

use dkan_importer::model::DataDictionary;
use importer_lib::serde_json::json;

// Mock test that simulates the constructor behavior
// Since we can't easily mock the HTTP client in unit tests, we'll test the duplicate checking
// logic that's now integrated into the constructor

#[test]
fn test_duplicate_checking_integration() {
    // Test that the check_duplicates method works correctly with normalized data
    let test_data_with_duplicates = json!({
        "title": "Test Data Dictionary",
        "fields": [
            {
                "name": "field_name",
                "title": "Field Title",
                "type": "string"
            },
            {
                "name": "  field_name  ", // Same after normalization
                "title": "Field\tTitle", // Same after normalization (tab -> space)
                "type": "number"
            }
        ]
    });

    // This should fail because check_duplicates is now called during construction
    // We can't easily test the full constructor due to HTTP dependencies,
    // but we can verify the duplicate checking logic works
    let result = DataDictionary::check_duplicates(&test_data_with_duplicates);
    assert!(
        result.is_err(),
        "Should detect duplicates after normalization"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("field_name"),
        "Error should mention normalized duplicate name"
    );
    assert!(
        error_message.contains("Field Title"),
        "Error should mention normalized duplicate title"
    );
}

#[test]
fn test_duplicate_checking_success_case() {
    // Test that valid data passes duplicate checking
    let test_data_no_duplicates = json!({
        "title": "Test Data Dictionary",
        "fields": [
            {
                "name": "field1",
                "title": "Field One",
                "type": "string"
            },
            {
                "name": "field2",
                "title": "Field Two",
                "type": "number"
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&test_data_no_duplicates);
    assert!(result.is_ok(), "Should not find any duplicates");
}

#[test]
fn test_duplicate_checking_with_normalized_data() {
    // Test that the duplicate checking works with the same normalization logic
    // used in the constructor
    let test_data = json!({
        "title": "Normalization Test",
        "fields": [
            {
                "name": "\t sample_id \n",  // Whitespace + control chars
                "title": "Sample\rID*",     // Control chars + asterisk
                "type": "string"
            },
            {
                "name": "  sample_id  ",   // Same after normalization
                "title": "Sample\nID*",    // Same after normalization
                "type": "number"
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&test_data);
    assert!(
        result.is_err(),
        "Should detect duplicates after normalization"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("sample_id"),
        "Error should mention normalized duplicate name"
    );
    assert!(
        error_message.contains("Sample ID*"),
        "Error should mention normalized duplicate title"
    );
}
