//! Unit tests for DataDictionary duplicate detection functionality
//! Tests for DataDictionary::check_duplicates static method

use dkan_importer::model::DataDictionary;
use serde_json::json;

mod common;

#[test]
fn test_data_dictionary_no_duplicates() {
    let data_dictionary = json!({
        "fields": [
            {
                "name": "field1",
                "title": "Field One",
                "type": "string"
            },
            {
                "name": "field2",
                "title": "Field Two",
                "type": "string"
            },
            {
                "name": "field3",
                "title": "Field Three",
                "type": "number"
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(result.is_ok(), "Should not find any duplicates");
}

#[test]
fn test_data_dictionary_duplicate_names() {
    let data_dictionary = json!({
        "fields": [
            {
                "name": "duplicate_name",
                "title": "First Title",
                "type": "string"
            },
            {
                "name": "other_name",
                "title": "Other Title",
                "type": "string"
            },
            {
                "name": "duplicate_name", // Duplicate name
                "title": "Second Title",
                "type": "number"
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(result.is_err(), "Should detect duplicate names");

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("duplicate_name"),
        "Error should mention the duplicate name"
    );
    assert!(
        error_message.contains("positions: 1, 3"),
        "Error should show correct positions (1-based)"
    );
    assert!(
        error_message.contains("Data dictionary contains duplicate fields"),
        "Error should have proper header"
    );
}

#[test]
fn test_data_dictionary_duplicate_titles() {
    let data_dictionary = json!({
        "fields": [
            {
                "name": "first_name",
                "title": "Duplicate Title",
                "type": "string"
            },
            {
                "name": "second_name",
                "title": "Unique Title",
                "type": "string"
            },
            {
                "name": "third_name",
                "title": "Duplicate Title", // Duplicate title
                "type": "number"
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(result.is_err(), "Should detect duplicate titles");

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("Duplicate Title"),
        "Error should mention the duplicate title"
    );
    assert!(
        error_message.contains("positions: 1, 3"),
        "Error should show correct positions"
    );
}

#[test]
fn test_data_dictionary_both_name_and_title_duplicates() {
    let data_dictionary = json!({
        "fields": [
            {
                "name": "duplicate_name",
                "title": "Duplicate Title",
                "type": "string"
            },
            {
                "name": "unique_name",
                "title": "Unique Title",
                "type": "string"
            },
            {
                "name": "duplicate_name", // Duplicate name
                "title": "Duplicate Title", // Duplicate title
                "type": "number"
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(result.is_err(), "Should detect both duplicates");

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("duplicate_name"),
        "Error should mention duplicate name"
    );
    assert!(
        error_message.contains("Duplicate Title"),
        "Error should mention duplicate title"
    );
}

#[test]
fn test_data_dictionary_normalized_duplicates() {
    let data_dictionary = json!({
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

    let result = DataDictionary::check_duplicates(&data_dictionary);
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
fn test_data_dictionary_empty_fields() {
    let data_dictionary = json!({
        "fields": []
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(result.is_ok(), "Empty fields should not have duplicates");
}

#[test]
fn test_data_dictionary_missing_fields_array() {
    let data_dictionary = json!({
        "not_fields": []
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(result.is_err(), "Should error when fields array is missing");

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("valid 'fields' array"),
        "Error should mention missing fields array"
    );
}

#[test]
fn test_data_dictionary_fields_without_names_or_titles() {
    let data_dictionary = json!({
        "fields": [
            {
                "type": "string"
                // No name or title
            },
            {
                "name": "valid_name",
                "type": "number"
                // No title
            },
            {
                "title": "Valid Title",
                "type": "boolean"
                // No name
            }
        ]
    });

    let result = DataDictionary::check_duplicates(&data_dictionary);
    assert!(
        result.is_ok(),
        "Should not error when some fields lack names/titles"
    );
}
