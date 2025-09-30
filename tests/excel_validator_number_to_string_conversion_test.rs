use dkan_importer::model::ExcelValidator;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[test]
fn test_number_to_string_conversion_when_schema_expects_string() {
    // Create a schema where a field is expected to be a string
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "title": "ID"
            },
            "count": {
                "type": "integer",
                "title": "Count"
            }
        },
        "required": ["id", "count"]
    });

    // Create title to name mapping
    let mut title_to_name_mapping = HashMap::new();
    title_to_name_mapping.insert("ID".to_string(), "id".to_string());
    title_to_name_mapping.insert("Count".to_string(), "count".to_string());

    // Create validator
    let validator = ExcelValidator::new(&schema, title_to_name_mapping).unwrap();

    // Create a row where ID is a number but schema expects string
    let mut json_obj = Map::new();
    json_obj.insert("id".to_string(), json!(12345)); // Number that should be converted to string
    json_obj.insert("count".to_string(), json!(100)); // Number that should remain number

    let row_value = Value::Object(json_obj);

    // Apply intelligent type coercion
    let coerced_value = validator.apply_intelligent_type_coercion(row_value);

    // Check that the ID field was converted to string
    let coerced_obj = coerced_value.as_object().unwrap();
    assert_eq!(coerced_obj.get("id").unwrap(), &json!("12345"));
    assert_eq!(coerced_obj.get("count").unwrap(), &json!(100));
}

#[test]
fn test_float_to_string_conversion_when_schema_expects_string() {
    // Create a schema where a field is expected to be a string
    let schema = json!({
        "type": "object",
        "properties": {
            "measurement": {
                "type": "string",
                "title": "Measurement"
            },
            "value": {
                "type": "number",
                "title": "Value"
            }
        },
        "required": ["measurement", "value"]
    });

    // Create title to name mapping
    let mut title_to_name_mapping = HashMap::new();
    title_to_name_mapping.insert("Measurement".to_string(), "measurement".to_string());
    title_to_name_mapping.insert("Value".to_string(), "value".to_string());

    // Create validator
    let validator = ExcelValidator::new(&schema, title_to_name_mapping).unwrap();

    // Create a row where measurement is a float but schema expects string
    let mut json_obj = Map::new();
    json_obj.insert("measurement".to_string(), json!(42.75)); // Float that should be converted to string
    json_obj.insert("value".to_string(), json!(100.5)); // Float that should remain number

    let row_value = Value::Object(json_obj);

    // Apply intelligent type coercion
    let coerced_value = validator.apply_intelligent_type_coercion(row_value);

    // Check that the measurement field was converted to string
    let coerced_obj = coerced_value.as_object().unwrap();
    assert_eq!(coerced_obj.get("measurement").unwrap(), &json!("42.75"));
    assert_eq!(coerced_obj.get("value").unwrap(), &json!(100.5));
}

#[test]
fn test_boolean_to_string_conversion_when_schema_expects_string() {
    // Create a schema where a field is expected to be a string
    let schema = json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "title": "Status"
            },
            "active": {
                "type": "boolean",
                "title": "Active"
            }
        },
        "required": ["status", "active"]
    });

    // Create title to name mapping
    let mut title_to_name_mapping = HashMap::new();
    title_to_name_mapping.insert("Status".to_string(), "status".to_string());
    title_to_name_mapping.insert("Active".to_string(), "active".to_string());

    // Create validator
    let validator = ExcelValidator::new(&schema, title_to_name_mapping).unwrap();

    // Create a row where status is a boolean but schema expects string
    let mut json_obj = Map::new();
    json_obj.insert("status".to_string(), json!(true)); // Boolean that should be converted to string
    json_obj.insert("active".to_string(), json!(false)); // Boolean that should remain boolean

    let row_value = Value::Object(json_obj);

    // Apply intelligent type coercion
    let coerced_value = validator.apply_intelligent_type_coercion(row_value);

    // Check that the status field was converted to string
    let coerced_obj = coerced_value.as_object().unwrap();
    assert_eq!(coerced_obj.get("status").unwrap(), &json!("true"));
    assert_eq!(coerced_obj.get("active").unwrap(), &json!(false));
}

#[test]
fn test_mixed_type_with_string_allows_number_conversion() {
    // Create a schema with mixed types that includes string
    let schema = json!({
        "type": "object",
        "properties": {
            "flexible_field": {
                "type": ["string", "number"],
                "title": "Flexible Field"
            },
            "strict_number": {
                "type": "number",
                "title": "Strict Number"
            }
        },
        "required": ["flexible_field", "strict_number"]
    });

    // Create title to name mapping
    let mut title_to_name_mapping = HashMap::new();
    title_to_name_mapping.insert("Flexible Field".to_string(), "flexible_field".to_string());
    title_to_name_mapping.insert("Strict Number".to_string(), "strict_number".to_string());

    // Create validator
    let validator = ExcelValidator::new(&schema, title_to_name_mapping).unwrap();

    // Create a row where flexible_field is a number but could be converted to string
    let mut json_obj = Map::new();
    json_obj.insert("flexible_field".to_string(), json!(42.5)); // Number that could be converted to string
    json_obj.insert("strict_number".to_string(), json!(100.0)); // Number that should remain number

    let row_value = Value::Object(json_obj);

    // Apply intelligent type coercion
    let coerced_value = validator.apply_intelligent_type_coercion(row_value);

    // Check that the flexible field was converted to string (since mixed type includes string)
    let coerced_obj = coerced_value.as_object().unwrap();
    assert_eq!(coerced_obj.get("flexible_field").unwrap(), &json!("42.5"));
    assert_eq!(coerced_obj.get("strict_number").unwrap(), &json!(100.0));
}

#[test]
fn test_no_conversion_when_schema_does_not_expect_string() {
    // Create a schema where no fields expect strings
    let schema = json!({
        "type": "object",
        "properties": {
            "count": {
                "type": "integer",
                "title": "Count"
            },
            "value": {
                "type": "number",
                "title": "Value"
            },
            "active": {
                "type": "boolean",
                "title": "Active"
            }
        },
        "required": ["count", "value", "active"]
    });

    // Create title to name mapping
    let mut title_to_name_mapping = HashMap::new();
    title_to_name_mapping.insert("Count".to_string(), "count".to_string());
    title_to_name_mapping.insert("Value".to_string(), "value".to_string());
    title_to_name_mapping.insert("Active".to_string(), "active".to_string());

    // Create validator
    let validator = ExcelValidator::new(&schema, title_to_name_mapping).unwrap();

    // Create a row with correct types
    let mut json_obj = Map::new();
    json_obj.insert("count".to_string(), json!(100));
    json_obj.insert("value".to_string(), json!(42.5));
    json_obj.insert("active".to_string(), json!(true));

    let row_value = Value::Object(json_obj.clone());

    // Apply intelligent type coercion
    let coerced_value = validator.apply_intelligent_type_coercion(row_value);

    // Check that no conversion occurred since schema doesn't expect strings
    let coerced_obj = coerced_value.as_object().unwrap();
    assert_eq!(coerced_obj.get("count").unwrap(), &json!(100));
    assert_eq!(coerced_obj.get("value").unwrap(), &json!(42.5));
    assert_eq!(coerced_obj.get("active").unwrap(), &json!(true));
}

#[test]
fn test_string_values_remain_unchanged() {
    // Create a schema where a field is expected to be a string
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "title": "Name"
            },
            "description": {
                "type": "string",
                "title": "Description"
            }
        },
        "required": ["name", "description"]
    });

    // Create title to name mapping
    let mut title_to_name_mapping = HashMap::new();
    title_to_name_mapping.insert("Name".to_string(), "name".to_string());
    title_to_name_mapping.insert("Description".to_string(), "description".to_string());

    // Create validator
    let validator = ExcelValidator::new(&schema, title_to_name_mapping).unwrap();

    // Create a row where fields are already strings
    let mut json_obj = Map::new();
    json_obj.insert("name".to_string(), json!("John Doe"));
    json_obj.insert("description".to_string(), json!("A test description"));

    let row_value = Value::Object(json_obj);

    // Apply intelligent type coercion
    let coerced_value = validator.apply_intelligent_type_coercion(row_value);

    // Check that string values remain unchanged
    let coerced_obj = coerced_value.as_object().unwrap();
    assert_eq!(coerced_obj.get("name").unwrap(), &json!("John Doe"));
    assert_eq!(
        coerced_obj.get("description").unwrap(),
        &json!("A test description")
    );
}
