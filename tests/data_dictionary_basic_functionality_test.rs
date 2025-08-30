use dkan_importer::model::data_dictionary::DataDictionary;
use serde_json::json;

#[test]
fn test_title_used_as_property_name() {
    let dkan_fields = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "sample_code",
                "title": "Sample Code",
                "type": "string",
                "description": "A sample code"
            },
            {
                "name": "date_of_sampling_start",
                "title": "Date of sampling start*",
                "type": "datetime",
                "description": "Start date"
            }
        ]
    });

    let schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_fields).unwrap();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Should use title as property name
    assert!(properties.contains_key("Sample Code"));
    assert!(properties.contains_key("Date of sampling start*"));

    // Should NOT use field name as property name
    assert!(!properties.contains_key("sample_code"));
    assert!(!properties.contains_key("date_of_sampling_start"));
}

#[test]
fn test_asterisk_in_title_indicates_required() {
    let dkan_fields = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "optional_field",
                "title": "Optional Field",
                "type": "string"
            },
            {
                "name": "required_field",
                "title": "Required Field*",
                "type": "string"
            }
        ]
    });

    let schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_fields).unwrap();
    let required = schema.get("required").unwrap().as_array().unwrap();

    // Only field with asterisk should be required
    assert_eq!(required.len(), 1);
    assert!(required.contains(&json!("Required Field*")));
    assert!(!required.contains(&json!("Optional Field")));
}

#[test]
fn test_fallback_to_name_when_no_title() {
    let dkan_fields = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "field_without_title",
                "type": "string"
            }
        ]
    });

    let schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_fields).unwrap();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Should use field name as property name when title is missing
    assert!(properties.contains_key("field_without_title"));
}

#[test]
fn test_title_with_newlines_preserved() {
    let dkan_fields = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "remark_1",
                "title": "Remark 1\nAnalytical Partner",
                "type": "string"
            }
        ]
    });

    let schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_fields).unwrap();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Should preserve full title including newlines
    assert!(properties.contains_key("Remark 1\nAnalytical Partner"));
    assert!(!properties.contains_key("remark_1"));
}

#[test]
fn test_mixed_title_and_no_title_fields() {
    let dkan_fields = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "with_title_field",
                "title": "With Title Field*",
                "type": "string"
            },
            {
                "name": "no_title_field",
                "type": "integer"
            }
        ]
    });

    let schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_fields).unwrap();
    let properties = schema.get("properties").unwrap().as_object().unwrap();
    let required = schema.get("required").unwrap().as_array().unwrap();

    // Should use title when available, name when not
    assert!(properties.contains_key("With Title Field*"));
    assert!(properties.contains_key("no_title_field"));
    assert!(!properties.contains_key("with_title_field"));

    // Required should use same property names
    assert!(required.contains(&json!("With Title Field*")));
    assert!(!required.contains(&json!("no_title_field")));
}
