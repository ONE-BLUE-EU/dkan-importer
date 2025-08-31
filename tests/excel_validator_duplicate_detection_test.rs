//! Unit tests for ExcelValidator duplicate detection functionality
//! Tests for ExcelValidator::check_header_duplicates static method

use dkan_importer::model::ExcelValidator;

mod common;

#[test]
fn test_excel_headers_no_duplicates() {
    let headers = vec![
        "Header A".to_string(),
        "Header B".to_string(),
        "Header C".to_string(),
    ];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(result.is_ok(), "Should not find any duplicate headers");
}

#[test]
fn test_excel_headers_with_duplicates() {
    let headers = vec![
        "Header A".to_string(),
        "Header B".to_string(),
        "Header A".to_string(), // Duplicate
    ];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(result.is_err(), "Should detect duplicate headers");

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("Header A"),
        "Error should mention duplicate header"
    );
    assert!(
        error_message.contains("column 1") && error_message.contains("column 3"),
        "Error should show correct column positions"
    );
    assert!(
        error_message.contains("duplicate column headers"),
        "Error should have proper header message"
    );
}

#[test]
fn test_excel_headers_multiple_duplicates() {
    let headers = vec![
        "Header A".to_string(),
        "Header B".to_string(),
        "Header C".to_string(),
        "Header A".to_string(), // Duplicate A
        "Header B".to_string(), // Duplicate B
    ];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(result.is_err(), "Should detect multiple duplicates");

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("Header A"),
        "Error should mention first duplicate"
    );
    assert!(
        error_message.contains("Header B"),
        "Error should mention second duplicate"
    );
    assert!(
        !error_message.contains("Header C"),
        "Error should not mention non-duplicate header"
    );
}

#[test]
fn test_excel_headers_normalized_duplicates() {
    let headers = vec![
        "Header A".to_string(),
        "  Header A  ".to_string(), // Same after normalization
        "Header\nB".to_string(),
        "Header B".to_string(), // Same after normalization (newline -> space)
    ];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(
        result.is_err(),
        "Should detect duplicates after normalization"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("Header A"),
        "Error should mention first normalized duplicate"
    );
    assert!(
        error_message.contains("Header B"),
        "Error should mention second normalized duplicate"
    );
}

#[test]
fn test_excel_headers_empty_list() {
    let headers: Vec<String> = vec![];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(
        result.is_ok(),
        "Empty headers list should not have duplicates"
    );
}

#[test]
fn test_excel_headers_single_header() {
    let headers = vec!["Single Header".to_string()];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(result.is_ok(), "Single header should not be a duplicate");
}

#[test]
fn test_excel_headers_case_sensitive() {
    // Our normalize_string function doesn't change case, so these should be different
    let headers = vec![
        "Header A".to_string(),
        "HEADER A".to_string(),
        "header a".to_string(),
    ];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(
        result.is_ok(),
        "Different cases should not be considered duplicates"
    );
}

#[test]
fn test_excel_headers_with_control_characters() {
    let headers = vec![
        "Header\tA".to_string(), // Tab will be normalized to space
        "Header A".to_string(),
        "Header\nB".to_string(), // Newline will be normalized to space
        "Header B".to_string(),
    ];

    let result = ExcelValidator::check_header_duplicates(&headers);
    assert!(
        result.is_err(),
        "Should detect duplicates after control character normalization"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("Header A"),
        "Should detect Header A duplicate"
    );
    assert!(
        error_message.contains("Header B"),
        "Should detect Header B duplicate"
    );
}
