//! Tests for core validator functionality

use calamine::Data;
use dkan_importer::model::ExcelValidator;
use serde_json::{json, Value};

mod common;

#[test]
fn test_excel_validator_new_success() {
    let schema = common::create_test_schema();
    let title_to_name_mapping = common::create_title_to_name_mapping_from_schema(&schema);
    let validator = ExcelValidator::new(&schema, title_to_name_mapping);
    assert!(validator.is_ok());
}

#[test]
fn test_convert_cell_to_json_string() {
    let validator = common::create_test_validator();

    let cell = Data::String("Hello World".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "name");

    assert_eq!(result, Value::String("Hello World".to_string()));
}

#[test]
fn test_backward_compatibility() {
    let validator = common::create_test_validator();

    // Test the new schema-aware method works for basic cases
    let cell = Data::String("test".to_string());
    let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "name");
    assert!(result.is_string());

    let cell2 = Data::Int(42);
    let result2 = validator.convert_cell_to_json_with_schema_awareness(&cell2, "age");
    assert_eq!(result2, json!(42));
}
