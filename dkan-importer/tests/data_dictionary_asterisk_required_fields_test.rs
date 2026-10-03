//! Tests for asterisk-based mandatory field detection with preservation
//! When either field name or title ends with '*', the field should be treated as mandatory
//! Original values should be preserved (no cleaning of asterisks)

use dkan_importer::model::data_dictionary::DataDictionary;
use importer_lib::serde_json::json;
mod common;

#[test]
fn test_asterisk_in_title_makes_field_required() {
    let dkan_schema = json!({
        "title": "Title Asterisk Test",
        "fields": [
            {
                "name": "regular_field",
                "title": "Regular Field",
                "type": "string"
            },
            {
                "name": "mandatory_field",
                "title": "Mandatory Field *",  // Asterisk in title
                "type": "string"
            },
            {
                "name": "number_field",
                "title": "Number Field *",     // Asterisk in title
                "type": "number"
            }
        ]
    });

    let normalized_schema =
        DataDictionary::normalize_field_data_for_tests(dkan_schema.clone()).unwrap();
    let json_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_schema).unwrap();

    // Check required fields (should contain property names, which are titles)
    let required = json_schema.get("required").unwrap().as_array().unwrap();
    assert!(
        required.contains(&json!("Mandatory Field*")),
        "Field with asterisk in title should be required"
    );
    assert!(
        required.contains(&json!("Number Field*")),
        "Number field with asterisk in title should be required"
    );
    assert!(
        !required.contains(&json!("Regular Field")),
        "Regular field without asterisk should not be required"
    );

    // Check that titles are preserved (asterisks NOT removed)
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();

    // Property keys should now be the normalized titles
    assert!(properties.contains_key("Mandatory Field*"));
    assert!(properties.contains_key("Number Field*"));
    assert!(properties.contains_key("Regular Field"));

    // And titles should be normalized in the property definitions
    assert_eq!(
        properties
            .get("Mandatory Field*")
            .unwrap()
            .get("title")
            .unwrap(),
        &json!("Mandatory Field*")
    );
    assert_eq!(
        properties
            .get("Number Field*")
            .unwrap()
            .get("title")
            .unwrap(),
        &json!("Number Field*")
    );
    assert_eq!(
        properties
            .get("Regular Field")
            .unwrap()
            .get("title")
            .unwrap(),
        &json!("Regular Field")
    );
}

#[test]
fn test_asterisk_in_name_makes_field_required() {
    let dkan_schema = json!({
        "title": "Name Asterisk Test",
        "fields": [
            {
                "name": "regular_field",
                "title": "Regular Field",
                "type": "string"
            },
            {
                "name": "mandatory_field*",    // Asterisk in name
                "title": "Mandatory Field",
                "type": "string"
            },
            {
                "name": "number_field*",       // Asterisk in name
                "title": "Number Field",
                "type": "number"
            }
        ]
    });

    let normalized_schema =
        DataDictionary::normalize_field_data_for_tests(dkan_schema.clone()).unwrap();
    let json_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_schema).unwrap();

    // Check required fields (property names should be titles, but since these have no titles, they use field names)
    let required = json_schema.get("required").unwrap().as_array().unwrap();
    assert!(
        required.contains(&json!("Mandatory Field")),
        "Field with asterisk in name should be required (using title as property name)"
    );
    assert!(
        required.contains(&json!("Number Field")),
        "Number field with asterisk in name should be required (using title as property name)"
    );
    assert!(
        !required.contains(&json!("Regular Field")),
        "Regular field without asterisk should not be required"
    );

    // Check that property keys use titles (since titles are available)
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();

    assert!(
        properties.contains_key("Mandatory Field"),
        "Property key should be the title"
    );
    assert!(
        properties.contains_key("Number Field"),
        "Property key should be the title"
    );
    assert!(
        properties.contains_key("Regular Field"),
        "Property key should be the title"
    );

    // Check that titles are preserved
    assert_eq!(
        properties
            .get("Mandatory Field")
            .unwrap()
            .get("title")
            .unwrap(),
        &json!("Mandatory Field")
    );
    assert_eq!(
        properties
            .get("Number Field")
            .unwrap()
            .get("title")
            .unwrap(),
        &json!("Number Field")
    );
}

#[test]
fn test_asterisk_in_both_name_and_title() {
    let dkan_schema = json!({
        "title": "Both Fields Asterisk Test",
        "fields": [
            {
                "name": "field_both*",         // Asterisk in name
                "title": "Both Fields *",     // Asterisk in title
                "type": "string"
            },
            {
                "name": "field_name_only*",   // Asterisk in name only
                "title": "Name Only",
                "type": "number"
            },
            {
                "name": "field_title_only",   // No asterisk in name
                "title": "Title Only *",     // Asterisk in title only
                "type": "boolean"
            },
            {
                "name": "field_neither",      // No asterisk in either
                "title": "Neither Field",
                "type": "string"
            }
        ]
    });

    let normalized_schema =
        DataDictionary::normalize_field_data_for_tests(dkan_schema.clone()).unwrap();
    let json_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_schema).unwrap();

    // All fields with asterisks (in either name or title) should be required
    // Property names are now titles, so required array contains titles
    let required = json_schema.get("required").unwrap().as_array().unwrap();
    assert_eq!(required.len(), 3, "Should have 3 required fields");
    assert!(required.contains(&json!("Both Fields*")));
    assert!(required.contains(&json!("Name Only")));
    assert!(required.contains(&json!("Title Only*")));
    assert!(!required.contains(&json!("Neither Field")));

    // Check that all original values are preserved
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();

    // Property keys should now be normalized titles
    assert!(properties.contains_key("Both Fields*"));
    assert!(properties.contains_key("Name Only"));
    assert!(properties.contains_key("Title Only*"));
    assert!(properties.contains_key("Neither Field"));

    // Titles normalized in property values
    assert_eq!(properties["Both Fields*"]["title"], json!("Both Fields*"));
    assert_eq!(properties["Name Only"]["title"], json!("Name Only"));
    assert_eq!(properties["Title Only*"]["title"], json!("Title Only*"));
    assert_eq!(properties["Neither Field"]["title"], json!("Neither Field"));
}

#[test]
fn test_asterisk_with_explicit_constraints_preserved() {
    let dkan_schema = json!({
        "title": "Mixed Constraints with Preservation",
        "fields": [
            {
                "name": "asterisk_name*",
                "title": "Asterisk Name",
                "type": "number"
            },
            {
                "name": "asterisk_title",
                "title": "Asterisk Title *",
                "type": "number"
            },
            {
                "name": "constraint_required",
                "title": "Constraint Required",
                "type": "number",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "both_methods*",
                "title": "Both Methods *",
                "type": "integer",
                "constraints": {
                    "required": true
                }
            },
            {
                "name": "asterisk_overrides*",
                "title": "Asterisk Override *",
                "type": "boolean",
                "constraints": {
                    "required": false  // This should be overridden by asterisk
                }
            }
        ]
    });

    let normalized_schema =
        DataDictionary::normalize_field_data_for_tests(dkan_schema.clone()).unwrap();
    let json_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_schema).unwrap();

    // Check required fields (should contain property names, which are titles)
    let required = json_schema.get("required").unwrap().as_array().unwrap();
    assert_eq!(required.len(), 5, "All fields should be required");
    assert!(required.contains(&json!("Asterisk Name")));
    assert!(required.contains(&json!("Asterisk Title*")));
    assert!(required.contains(&json!("Constraint Required")));
    assert!(required.contains(&json!("Both Methods*")));
    assert!(required.contains(&json!("Asterisk Override*")));

    // Check that all names and titles are preserved
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();
    assert_eq!(properties["Asterisk Name"]["title"], json!("Asterisk Name"));
    assert_eq!(
        properties["Asterisk Title*"]["title"],
        json!("Asterisk Title*")
    );
    assert_eq!(
        properties["Constraint Required"]["title"],
        json!("Constraint Required")
    );
    assert_eq!(properties["Both Methods*"]["title"], json!("Both Methods*"));
    assert_eq!(
        properties["Asterisk Override*"]["title"],
        json!("Asterisk Override*")
    );

    // Check that all mandatory fields have simple types (not union with null)
    // Properties are now named after titles, not field names
    assert_eq!(properties["Asterisk Name"]["type"], json!("number"));
    assert_eq!(properties["Asterisk Title*"]["type"], json!("number"));
    assert_eq!(properties["Constraint Required"]["type"], json!("number"));
    assert_eq!(properties["Both Methods*"]["type"], json!("integer"));
    assert_eq!(properties["Asterisk Override*"]["type"], json!("boolean"));
}

#[test]
fn test_asterisk_edge_cases_preserved() {
    let dkan_schema = json!({
        "title": "Edge Cases with Preservation",
        "fields": [
            {
                "name": "multiple_asterisks***",
                "title": "Multiple ***",
                "type": "string"
            },
            {
                "name": "spaces_before_asterisk ",  // Space at end, no asterisk
                "title": "Spaces Before * ",        // Space after asterisk
                "type": "string"
            },
            {
                "name": "asterisk_middle*middle",   // Asterisk in middle of name
                "title": "Asterisk * In Middle",   // Asterisk in middle of title
                "type": "string"
            },
            {
                "name": "name_ends*",              // Name ends with asterisk
                "title": "Title Does Not",        // Title does not end with asterisk
                "type": "string"
            },
            {
                "name": "name_does_not",           // Name does not end with asterisk
                "title": "Title Ends *",          // Title ends with asterisk
                "type": "string"
            },
            {
                "name": "no_title_field*",        // Name has asterisk, no title
                "type": "string"
            },
            {
                "name": "empty_title_field",
                "title": "",
                "type": "string"
            }
        ]
    });

    let normalized_schema =
        DataDictionary::normalize_field_data_for_tests(dkan_schema.clone()).unwrap();
    let json_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_schema).unwrap();

    let required = json_schema.get("required").unwrap().as_array().unwrap();
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();

    // Multiple asterisks at end should be treated as mandatory (property name is title)
    assert!(required.contains(&json!("Multiple***")));
    assert_eq!(properties["Multiple***"]["title"], json!("Multiple***"));

    // Space after asterisk should work (using title as property name, after normalization trailing spaces are trimmed)
    assert!(required.contains(&json!("Spaces Before*")));
    assert_eq!(
        properties["Spaces Before*"]["title"],
        json!("Spaces Before*")
    );

    // Asterisk not at end should NOT make field mandatory (property name is title)
    // "Asterisk * In Middle" - this doesn't end with *, so should not be required
    assert!(!required.contains(&json!("Asterisk * In Middle")));
    assert_eq!(
        properties["Asterisk * In Middle"]["title"],
        json!("Asterisk * In Middle")
    );

    // Name ends with asterisk should be required (using title as property name)
    assert!(required.contains(&json!("Title Does Not")));
    assert_eq!(
        properties["Title Does Not"]["title"],
        json!("Title Does Not")
    );

    // Title ends with asterisk should be required (using title as property name)
    assert!(required.contains(&json!("Title Ends*")));
    assert_eq!(properties["Title Ends*"]["title"], json!("Title Ends*"));

    // No title field but name has asterisk should be required
    assert!(required.contains(&json!("no_title_field*")));

    // Empty title should not be required
    assert!(!required.contains(&json!("empty_title_field")));
}

#[test]
fn test_type_handling_with_preserved_asterisks() {
    let dkan_schema = json!({
        "title": "Type Handling with Preserved Asterisks",
        "fields": [
            {
                "name": "mandatory_string*",
                "title": "Mandatory String",
                "type": "string"
            },
            {
                "name": "mandatory_number",
                "title": "Mandatory Number *",
                "type": "number"
            },
            {
                "name": "optional_number",
                "title": "Optional Number",
                "type": "number"
            },
            {
                "name": "optional_boolean",
                "title": "Optional Boolean",
                "type": "boolean"
            }
        ]
    });

    let normalized_schema =
        DataDictionary::normalize_field_data_for_tests(dkan_schema.clone()).unwrap();
    let json_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_schema).unwrap();
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();

    // Mandatory fields should have simple types (using title-based property names)
    assert_eq!(properties["Mandatory String"]["type"], json!("string"));
    assert_eq!(properties["Mandatory Number*"]["type"], json!("number"));

    // Optional fields should have union types with null
    assert_eq!(
        properties["Optional Number"]["type"],
        json!(["number", "null"])
    );
    assert_eq!(
        properties["Optional Boolean"]["type"],
        json!(["boolean", "null"])
    );

    // Verify required fields (using title-based property names)
    let required = json_schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("Mandatory String")));
    assert!(required.contains(&json!("Mandatory Number*")));
    assert!(!required.contains(&json!("Optional Number")));
    assert!(!required.contains(&json!("Optional Boolean")));
}
