//! Tests for DKAN schema conversion functionality

use dkan_importer::model::DataDictionary;
use serde_json::json;

#[test]
fn test_datetime_with_dkan_format() {
    let dkan_schema = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "date_field",
                "type": "datetime",
                "format": "%Y/%m/%d",
                "title": "Date Field",
                "description": "A custom date field"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let props = &json_schema["properties"]["date_field"];
    assert_eq!(props["type"], "string");
    assert_eq!(props["format"], "%Y/%m/%d");
    assert_eq!(props["dkan_format"], "%Y/%m/%d");
}

#[test]
fn test_datetime_with_default_format() {
    let dkan_schema = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "date_field",
                "type": "datetime",
                "format": "default",
                "title": "Date Field",
                "description": "A default date field"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let props = &json_schema["properties"]["date_field"];
    assert_eq!(props["type"], "string");
    assert_eq!(props["format"], "date-time");
    assert!(props.get("dkan_format").is_none());
}

#[test]
fn test_dkan_schema_conversion_basic() {
    let dkan_schema = json!({
        "title": "Sample Schema",
        "fields": [
            {
                "name": "id",
                "type": "integer",
                "title": "ID",
                "description": "Unique identifier"
            },
            {
                "name": "name",
                "type": "string",
                "title": "Name",
                "description": "Full name",
                "constraints": {
                    "required": true,
                    "minLength": 1,
                    "maxLength": 100
                }
            },
            {
                "name": "score",
                "type": "number",
                "title": "Score",
                "constraints": {
                    "minimum": 0.0,
                    "maximum": 100.0
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();

    // Check schema structure
    assert_eq!(json_schema["type"], "object");
    assert_eq!(json_schema["title"], "Sample Schema");
    assert_eq!(json_schema["additionalProperties"], false);

    // Check properties
    let properties = &json_schema["properties"];
    assert_eq!(properties["id"]["type"], "integer");
    assert_eq!(properties["name"]["type"], "string");
    assert_eq!(properties["name"]["minLength"], 1);
    assert_eq!(properties["name"]["maxLength"], 100);
    assert_eq!(properties["score"]["type"], "number");
    assert_eq!(properties["score"]["minimum"], 0.0);
    assert_eq!(properties["score"]["maximum"], 100.0);

    // Check required fields
    let required = &json_schema["required"];
    assert!(required.as_array().unwrap().contains(&json!("name")));
}
