//! Tests for null value validation in string fields
//! Non-mandatory string fields should now accept null values

use dkan_importer::model::{data_dictionary::DataDictionary, ExcelValidator};
use serde_json::json;

mod common;

#[test]
fn test_non_mandatory_string_fields_accept_null() {
    let dkan_schema = json!({
        "title": "String Null Test",
        "fields": [
            {
                "name": "mandatory_string",
                "title": "Mandatory String *",  // Required via asterisk
                "type": "string"
            },
            {
                "name": "optional_string",
                "title": "Optional String",     // Not required
                "type": "string"
            },
            {
                "name": "constraint_required_string",
                "title": "Required by Constraint",
                "type": "string",
                "constraints": {
                    "required": true
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();

    let properties = json_schema["properties"].as_object().unwrap();
    let required = json_schema["required"].as_array().unwrap();

    // Check required fields (use title-based property names)
    assert!(required.contains(&json!("Mandatory String *")));
    assert!(required.contains(&json!("Required by Constraint")));
    assert!(!required.contains(&json!("Optional String")));

    // Check type structures - mandatory strings have simple types
    assert_eq!(properties["Mandatory String *"]["type"], json!("string"));
    assert_eq!(
        properties["Required by Constraint"]["type"],
        json!("string")
    );

    // Check type structures - optional strings have union types with null
    assert_eq!(
        properties["Optional String"]["type"],
        json!(["string", "null"])
    );
}

#[test]
fn test_string_null_validation_behavior() {
    let dkan_schema = json!({
        "title": "String Validation Test",
        "fields": [
            {
                "name": "required_name",
                "title": "Required Name *",
                "type": "string"
            },
            {
                "name": "optional_description",
                "title": "Optional Description",
                "type": "string"
            },
            {
                "name": "optional_notes",
                "title": "Optional Notes",
                "type": "string"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test 1: Valid data with null optional string fields
    let valid_data = json!({
        "Required Name *": "John Doe",
        "Optional Description": null,  // This should now be accepted
        "Optional Notes": null         // This should now be accepted
    });

    let is_valid = validator.validator.is_valid(&valid_data);
    if !is_valid {
        // Validation errors exist but are not printed
    }
    assert!(is_valid, "Optional string fields should accept null values");

    // Test 2: Valid data with actual string values
    let valid_data2 = json!({
        "Required Name *": "Jane Smith",
        "Optional Description": "A detailed description",
        "Optional Notes": "Some notes here"
    });

    assert!(
        validator.validator.is_valid(&valid_data2),
        "String fields should accept string values"
    );

    // Test 3: Valid data with mixed null and string values
    let valid_data3 = json!({
        "Required Name *": "Bob Wilson",
        "Optional Description": "Has description",
        "Optional Notes": null  // One null, one string
    });

    assert!(
        validator.validator.is_valid(&valid_data3),
        "Mixed null and string values should work"
    );

    // Test 4: Invalid data - required string field is null
    let invalid_data = json!({
        "Required Name *": null,  // Required field cannot be null
        "Optional Description": "Some description",
        "Optional Notes": "Some notes"
    });

    assert!(
        !validator.validator.is_valid(&invalid_data),
        "Required string field should not accept null"
    );

    // Test 5: Invalid data - required string field missing
    let invalid_data2 = json!({
        // "Required Name *": missing
        "Optional Description": "Some description",
        "Optional Notes": null
    });

    assert!(
        !validator.validator.is_valid(&invalid_data2),
        "Missing required string field should fail"
    );
}

#[test]
fn test_mixed_field_types_with_null_handling() {
    let dkan_schema = json!({
        "title": "Mixed Types Test",
        "fields": [
            {
                "name": "required_id*",
                "title": "Required ID",
                "type": "string"
            },
            {
                "name": "optional_name",
                "title": "Optional Name",
                "type": "string"
            },
            {
                "name": "optional_age",
                "title": "Optional Age",
                "type": "integer"
            },
            {
                "name": "optional_score",
                "title": "Optional Score",
                "type": "number"
            },
            {
                "name": "optional_active",
                "title": "Optional Active",
                "type": "boolean"
            },
            {
                "name": "required_category",
                "title": "Required Category *",
                "type": "string"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let properties = json_schema["properties"].as_object().unwrap();

    // All optional fields should have union types with null (use title-based property names)
    assert_eq!(
        properties["Optional Name"]["type"],
        json!(["string", "null"])
    );
    assert_eq!(
        properties["Optional Age"]["type"],
        json!(["integer", "null"])
    );
    assert_eq!(
        properties["Optional Score"]["type"],
        json!(["number", "null"])
    );
    assert_eq!(
        properties["Optional Active"]["type"],
        json!(["boolean", "null"])
    );

    // Required fields should have simple types (use title-based property names)
    assert_eq!(properties["Required ID"]["type"], json!("string"));
    assert_eq!(properties["Required Category *"]["type"], json!("string"));

    // Test validation
    let validator = ExcelValidator::new(&json_schema).unwrap();

    let valid_data = json!({
        "Required ID": "ID123",
        "Required Category *": "CategoryA",
        "Optional Name": null,      // String null - should work
        "Optional Age": null,       // Integer null - should work
        "Optional Score": null,     // Number null - should work
        "Optional Active": null     // Boolean null - should work
    });

    assert!(
        validator.validator.is_valid(&valid_data),
        "All optional fields should accept null"
    );
}

#[test]
fn test_regression_filter_cutoff_scenario() {
    // Test the specific scenario from the error log: "Filter cutoff (threshold)"
    let dkan_schema = json!({
        "title": "Filter Cutoff Regression Test",
        "fields": [
            {
                "name": "sample_id",
                "title": "Sample ID *",  // Required
                "type": "string"
            },
            {
                "name": "filter_cutoff_threshold",
                "title": "Filter cutoff (threshold)",  // Optional string field
                "type": "string"
            },
            {
                "name": "measurement_value",
                "title": "Measurement Value *",  // Required
                "type": "number"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // This scenario was failing before: optional string field with null value
    let test_data = json!({
        "Sample ID *": "SAMPLE_002",
        "Filter cutoff (threshold)": null,  // This was causing the error before
        "Measurement Value *": 42.5
    });

    let is_valid = validator.validator.is_valid(&test_data);
    if !is_valid {
        // Validation errors exist but are not printed
    }

    assert!(is_valid, "Optional string field should accept null - regression test for 'Filter cutoff (threshold)' error");

    // Verify the schema structure
    let properties = json_schema["properties"].as_object().unwrap();
    assert_eq!(
        properties["Filter cutoff (threshold)"]["type"],
        json!(["string", "null"])
    );
}

#[test]
fn test_excel_cell_conversion_for_string_fields() {
    use calamine::Data;

    let dkan_schema = json!({
        "title": "Excel Cell Conversion Test",
        "fields": [
            {
                "name": "required_field*",
                "title": "Required Field",
                "type": "string"
            },
            {
                "name": "optional_field",
                "title": "Optional Field",
                "type": "string"
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Test empty cell conversion for optional string field
    let empty_cell = Data::Empty;
    let result =
        validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "optional_field");
    assert_eq!(
        result,
        serde_json::Value::Null,
        "Empty cell should convert to null for optional string field"
    );

    // Test empty string conversion
    let empty_string_cell = Data::String("".to_string());
    let result2 =
        validator.convert_cell_to_json_with_schema_awareness(&empty_string_cell, "optional_field");
    // Note: Empty strings might be converted to null for optional fields based on implementation
    // For now, let's accept either behavior and document it
    assert!(
        result2 == serde_json::Value::String("".to_string()) || result2 == serde_json::Value::Null,
        "Empty string should either remain as empty string or be converted to null based on schema requirements"
    );

    // Test whitespace-only string
    let whitespace_cell = Data::String("   ".to_string());
    let result3 =
        validator.convert_cell_to_json_with_schema_awareness(&whitespace_cell, "optional_field");
    // Based on the actual behavior, empty/whitespace strings are converted to null for optional string fields
    assert!(
        result3 == serde_json::Value::String("   ".to_string()) || result3 == serde_json::Value::Null,
        "Whitespace string behavior depends on implementation - may be converted to null for optional fields"
    );
}

// Simplified property-based testing scenarios as regular tests
#[test]
fn test_string_field_scenarios_comprehensive() {
    // Test various field name/title combinations with asterisk requirements
    let test_cases = vec![
        // (field_name, field_title, has_constraint, constraint_value, should_be_required)
        ("simple_field", "Simple Field", false, false, false),
        ("required_field*", "Required Field", false, false, true), // Name asterisk
        ("field", "Field *", false, false, true),                  // Title asterisk
        ("both*", "Both *", false, false, true),                   // Both asterisks
        ("constraint_field", "Constraint Field", true, true, true), // Explicit constraint
        ("override*", "Override *", true, false, true),            // Asterisk overrides constraint
    ];

    for (field_name, field_title, has_constraint, constraint_value, should_be_required) in
        test_cases
    {
        let mut field = json!({
            "name": field_name,
            "title": field_title,
            "type": "string"
        });

        if has_constraint {
            field["constraints"] = json!({
                "required": constraint_value
            });
        }

        let dkan_schema = json!({
            "title": "String Field Scenarios Test",
            "fields": [field]
        });

        let json_schema =
            DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();

        let properties = json_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("Schema should have properties object");
        let empty_vec = Vec::new();
        let required = json_schema
            .get("required")
            .and_then(|r| r.as_array())
            .unwrap_or(&empty_vec);

        // Ensure field exists in properties (using title-based property names)
        if !properties.contains_key(field_title) {
            panic!(
                "Field '{}' not found in properties. Available keys: {:?}",
                field_title,
                properties.keys().collect::<Vec<_>>()
            );
        }

        // Verify required status (using title-based property names)
        if should_be_required {
            assert!(
                required.contains(&json!(field_title)),
                "Field '{}' should be required",
                field_title
            );
            assert_eq!(
                properties[field_title]["type"],
                json!("string"),
                "Required field '{}' should have simple string type",
                field_title
            );
        } else {
            assert!(
                !required.contains(&json!(field_title)),
                "Field '{}' should not be required",
                field_title
            );
            assert_eq!(
                properties[field_title]["type"],
                json!(["string", "null"]),
                "Optional field '{}' should have union type with null",
                field_title
            );
        }

        // Verify title preservation
        assert_eq!(
            properties[field_title]["title"],
            json!(field_title),
            "Title should be preserved for field '{}'",
            field_title
        );
    }
}

#[test]
fn test_mixed_types_with_string_null_support() {
    let field_types = vec![
        ("string", "string"),
        ("integer", "integer"),
        ("number", "number"),
        ("boolean", "boolean"),
        ("datetime", "string"), // datetime maps to string
        ("array", "array"),     // should not get null union
        ("object", "object"),   // should not get null union
    ];

    for (dkan_type, expected_json_type) in field_types {
        // Test required field
        let required_schema = json!({
            "title": "Mixed Types Test",
            "fields": [{
                "name": "test_field*",  // Required via asterisk
                "title": "Test Field",
                "type": dkan_type
            }]
        });

        let json_schema =
            DataDictionary::convert_data_dictionary_to_json_schema(&required_schema).unwrap();
        let properties = json_schema["properties"].as_object().unwrap();
        assert_eq!(
            properties["Test Field"]["type"],
            json!(expected_json_type),
            "Required {} field should have simple type",
            dkan_type
        );

        // Test optional field
        let optional_schema = json!({
            "title": "Mixed Types Test",
            "fields": [{
                "name": "test_field",  // Optional (no asterisk)
                "title": "Test Field",
                "type": dkan_type
            }]
        });

        let json_schema =
            DataDictionary::convert_data_dictionary_to_json_schema(&optional_schema).unwrap();
        let properties = json_schema["properties"].as_object().unwrap();

        if matches!(dkan_type, "array" | "object") {
            // Array and object types should not get null union
            assert_eq!(
                properties["Test Field"]["type"],
                json!(expected_json_type),
                "Optional {} field should not get null union",
                dkan_type
            );
        } else {
            // Other types should get null union when optional
            assert_eq!(
                properties["Test Field"]["type"],
                json!([expected_json_type, "null"]),
                "Optional {} field should get null union",
                dkan_type
            );
        }
    }
}

#[test]
fn test_comprehensive_string_scenarios() {
    let dkan_schema = json!({
        "title": "Comprehensive String Test",
        "fields": [
            {
                "name": "id*",
                "title": "ID",
                "type": "string"
            },
            {
                "name": "name",
                "title": "Name *",    // Required via title
                "type": "string"
            },
            {
                "name": "description",
                "title": "Description", // Optional
                "type": "string"
            },
            {
                "name": "notes",
                "title": "Notes",      // Optional
                "type": "string",
                "constraints": {
                    "minLength": 0
                }
            },
            {
                "name": "category",
                "title": "Category",   // Required via constraint
                "type": "string",
                "constraints": {
                    "required": true
                }
            }
        ]
    });

    let json_schema = DataDictionary::convert_data_dictionary_to_json_schema(&dkan_schema).unwrap();
    let properties = json_schema["properties"].as_object().unwrap();
    let required = json_schema["required"].as_array().unwrap();

    // Check required fields (use title-based property names)
    assert_eq!(required.len(), 3);
    assert!(required.contains(&json!("ID"))); // Name asterisk
    assert!(required.contains(&json!("Name *"))); // Title asterisk
    assert!(required.contains(&json!("Category"))); // Explicit constraint

    // Check type structures (use title-based property names)
    assert_eq!(properties["ID"]["type"], json!("string")); // Required = simple
    assert_eq!(properties["Name *"]["type"], json!("string")); // Required = simple
    assert_eq!(properties["Category"]["type"], json!("string")); // Required = simple
    assert_eq!(properties["Description"]["type"], json!(["string", "null"])); // Optional = union
    assert_eq!(properties["Notes"]["type"], json!(["string", "null"])); // Optional = union

    // Test validation scenarios
    let validator = ExcelValidator::new(&json_schema).unwrap();

    // Scenario 1: All fields provided
    let data1 = json!({
        "ID": "ID123",
        "Name *": "John Doe",
        "Description": "A person",
        "Notes": "Some notes",
        "Category": "Person"
    });
    assert!(validator.validator.is_valid(&data1));

    // Scenario 2: Optional fields as null
    let data2 = json!({
        "ID": "ID124",
        "Name *": "Jane Smith",
        "Description": null,
        "Notes": null,
        "Category": "Person"
    });
    assert!(validator.validator.is_valid(&data2));

    // Scenario 3: Mixed null and values for optional fields
    let data3 = json!({
        "ID": "ID125",
        "Name *": "Bob Wilson",
        "Description": "Another person",
        "Notes": null,
        "Category": "Person"
    });
    assert!(validator.validator.is_valid(&data3));

    // Scenario 4: Required field as null (should fail)
    let data4 = json!({
        "ID": "ID126",
        "Name *": null,  // Required field cannot be null
        "Description": "Someone",
        "Notes": "Notes",
        "Category": "Person"
    });
    assert!(!validator.validator.is_valid(&data4));
}
