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
    // Properties are now named after titles
    let props = &json_schema["properties"]["Date Field"];
    assert_eq!(props["type"], json!(["string", "null"])); // Now datetime fields get null union when optional
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
    // Properties are now named after titles
    let props = &json_schema["properties"]["Date Field"];
    assert_eq!(props["type"], json!(["string", "null"])); // Now datetime fields get null union when optional
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

    // id field is not mandatory (no required constraint), so it should allow null
    // Properties are now named after titles
    assert_eq!(properties["ID"]["type"], json!(["integer", "null"]));

    // name field is mandatory (required: true), so it should be just string
    assert_eq!(properties["Name"]["type"], "string");
    assert_eq!(properties["Name"]["minLength"], 1);
    assert_eq!(properties["Name"]["maxLength"], 100);

    // score field is not mandatory (no required constraint), so it should allow null
    assert_eq!(properties["Score"]["type"], json!(["number", "null"]));
    assert_eq!(properties["Score"]["minimum"], 0.0);
    assert_eq!(properties["Score"]["maximum"], 100.0);

    // Check required fields (properties are now named after titles)
    let required = &json_schema["required"];
    assert!(required.as_array().unwrap().contains(&json!("Name")));
}
