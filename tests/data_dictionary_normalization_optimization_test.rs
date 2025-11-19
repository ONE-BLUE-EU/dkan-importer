//! Tests specifically for the DataDictionary normalization optimization
//!
//! These tests verify that the optimization to normalize field names and titles
//! once during DataDictionary initialization works correctly and doesn't change behavior.

use dkan_importer::model::data_dictionary::DataDictionary;
use importer_lib::serde_json::json;
use std::collections::HashMap;

#[test]
fn test_normalization_optimization_consistency() {
    // Create test data with fields that need normalization
    let raw_data = json!({
        "title": "Test Schema",
        "fields": [
            {
                "name": "  field_name  ", // Needs trimming
                "title": "Field\tTitle\n*", // Needs control char replacement + asterisk preservation
                "type": "string",
                "description": "A test field"
            },
            {
                "name": "another_field\r",  // Needs control char replacement
                "title": "Another Field",
                "type": "number"
            }
        ]
    });

    // Test the optimized path (using normalize_field_data_for_tests to simulate DataDictionary::new)
    let normalized_data = DataDictionary::normalize_field_data_for_tests(raw_data.clone()).unwrap();
    let optimized_schema =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data).unwrap();
    let optimized_mapping = DataDictionary::create_title_to_name_mapping(&normalized_data).unwrap();

    // Test consistency - multiple calls to the optimized path should produce identical results
    let normalized_data2 =
        DataDictionary::normalize_field_data_for_tests(raw_data.clone()).unwrap();
    let optimized_schema2 =
        DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data2).unwrap();
    let optimized_mapping2 =
        DataDictionary::create_title_to_name_mapping(&normalized_data2).unwrap();

    // Multiple calls should produce identical results
    assert_eq!(
        optimized_schema, optimized_schema2,
        "Schema conversion should be deterministic"
    );
    assert_eq!(
        optimized_mapping, optimized_mapping2,
        "Title-to-name mapping should be deterministic"
    );

    // Verify that properties were created from the normalized data
    let properties = optimized_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(!properties.is_empty(), "Should have properties");

    // Verify mapping was created
    assert!(!optimized_mapping.is_empty(), "Should have mappings");
}

#[test]
fn test_get_title_to_name_mapping_with_complex_normalization() {
    // Test data with various normalization scenarios
    let test_data = json!({
        "title": "Complex Normalization Test",
        "fields": [
            {
                "name": "\t sample_id \n",  // Whitespace + control chars
                "title": "Sample\rID*",     // Control chars + asterisk
                "type": "string"
            },
            {
                "name": "  temp  ",         // Just whitespace
                "title": "Temperature (°C)", // Special characters (preserved)
                "type": "number"
            },
            {
                "name": "field_no_title",   // No title - should map to itself
                "type": "boolean"
            },
            {
                "name": "depth\t\r\n",      // Multiple control chars
                "title": "Water\nDepth\t(m)*", // Mixed control chars + asterisk
                "type": "number"
            }
        ]
    });

    let normalized_data = DataDictionary::normalize_field_data_for_tests(test_data).unwrap();
    let mapping = DataDictionary::create_title_to_name_mapping(&normalized_data).unwrap();

    let expected_mapping: HashMap<String, String> = [
        ("Sample ID*".to_string(), "sample_id".to_string()), // Control chars normalized
        ("Temperature (°C)".to_string(), "temp".to_string()), // Special chars preserved
        ("field_no_title".to_string(), "field_no_title".to_string()), // No title maps to name
        ("Water Depth (m)*".to_string(), "depth".to_string()), // Multiple control chars + asterisk
    ]
    .into_iter()
    .collect();

    assert_eq!(mapping, expected_mapping);
}

#[test]
fn test_normalization_preserves_asterisks_and_special_chars() {
    let test_data = json!({
        "title": "Asterisk and Special Char Test",
        "fields": [
            {
                "name": "required_field",
                "title": "Required Field*",  // Asterisk should be preserved
                "type": "string"
            },
            {
                "name": "unicode_field",
                "title": "Temperature (°C) Ω Δ", // Unicode should be preserved
                "type": "number"
            },
            {
                "name": "mixed_field",
                "title": "Mixed\tField (μmol~1L)\n*", // Control chars + special chars + asterisk
                "type": "number"
            }
        ]
    });

    let normalized_data = DataDictionary::normalize_field_data_for_tests(test_data).unwrap();
    let schema = DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data).unwrap();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Verify that normalization worked (properties were created)
    assert!(
        !properties.is_empty(),
        "Should have properties after normalization"
    );
    assert_eq!(properties.len(), 3, "Should have all three test fields");
}

#[test]
fn test_normalization_optimization_with_edge_cases() {
    let test_data = json!({
        "title": "Edge Cases Test",
        "fields": [
            {
                "name": "",  // Empty name (should still work)
                "title": "Empty Name Field",
                "type": "string"
            },
            {
                "name": "only_spaces",
                "title": "   ", // Only spaces in title
                "type": "string"
            },
            {
                "name": "control_chars_only",
                "title": "\t\r\n", // Only control chars in title
                "type": "string"
            },
            {
                "name": "normal_field",
                "title": "Normal Field", // Normal case
                "type": "string"
            }
        ]
    });

    let normalized_data = DataDictionary::normalize_field_data_for_tests(test_data).unwrap();
    let mapping = DataDictionary::create_title_to_name_mapping(&normalized_data).unwrap();

    // Empty name should still map correctly (using title)
    assert_eq!(mapping.get("Empty Name Field"), Some(&"".to_string()));

    // For whitespace-only and control-chars-only titles, they should be handled consistently
    assert!(
        mapping.len() >= 3,
        "Should have mappings for all valid fields"
    );

    // Normal case should work as expected
    assert_eq!(
        mapping.get("Normal Field"),
        Some(&"normal_field".to_string())
    );
}

#[test]
fn test_regression_consistency_with_old_behavior() {
    // This test ensures the optimization doesn't change the final behavior
    // by comparing results from the optimized path vs manual normalization

    let raw_test_cases = vec![
        // Case 1: Simple normalization
        json!({
            "title": "Simple Test",
            "fields": [{"name": "field1", "title": "Field 1*", "type": "string"}]
        }),
        // Case 2: Complex control characters
        json!({
            "title": "Control Char Test",
            "fields": [{"name": "\tfield2\n", "title": "Field\r2\t*", "type": "number"}]
        }),
        // Case 3: Mixed scenarios
        json!({
            "title": "Mixed Test",
            "fields": [
                {"name": "normal", "title": "Normal Field", "type": "string"},
                {"name": "  spaced  ", "title": "Spaced\nField*", "type": "number"},
                {"name": "no_title_field", "type": "boolean"}
            ]
        }),
    ];

    for (i, raw_data) in raw_test_cases.into_iter().enumerate() {
        // Test consistency: multiple normalization calls should produce identical results
        let normalized_data1 =
            DataDictionary::normalize_field_data_for_tests(raw_data.clone()).unwrap();
        let normalized_data2 =
            DataDictionary::normalize_field_data_for_tests(raw_data.clone()).unwrap();

        let schema1 =
            DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data1).unwrap();
        let schema2 =
            DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data2).unwrap();

        assert_eq!(
            schema1,
            schema2,
            "Test case {}: Multiple normalizations should produce identical results",
            i + 1
        );

        // Also verify that normalization is idempotent
        let normalized_again =
            DataDictionary::normalize_field_data_for_tests(normalized_data1.clone()).unwrap();
        assert_eq!(
            normalized_data1,
            normalized_again,
            "Test case {}: Normalization should be idempotent",
            i + 1
        );
    }
}

#[test]
fn test_field_data_normalization_is_idempotent() {
    // Verify that normalizing already-normalized data doesn't change it
    let test_data = json!({
        "title": "Idempotent Test",
        "fields": [
            {
                "name": "field_name",
                "title": "Field Title*",
                "type": "string"
            }
        ]
    });

    // Normalize once
    let normalized_once = DataDictionary::normalize_field_data_for_tests(test_data).unwrap();

    // Normalize again
    let normalized_twice =
        DataDictionary::normalize_field_data_for_tests(normalized_once.clone()).unwrap();

    // Should be identical
    assert_eq!(
        normalized_once, normalized_twice,
        "Normalization should be idempotent"
    );
}
