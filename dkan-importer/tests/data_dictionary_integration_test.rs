//! Integration tests for DataDictionary with full initialization flow
//!
//! These tests verify that the complete DataDictionary::new flow works correctly
//! with the normalization optimization.

use dkan_importer::model::data_dictionary::DataDictionary;
use importer_lib::serde_json::json;
use std::collections::HashMap;

#[cfg(test)]
mod mock_server_tests {
    use super::*;
    // use reqwest::blocking::Client; // For future HTTP mock tests

    // Note: These tests would ideally use a mock HTTP server, but for now
    // we'll test the core logic with the static methods since DataDictionary::new
    // requires an HTTP client and server setup.

    #[test]
    fn test_complete_flow_normalization_consistency() {
        // Test the complete flow using the static methods to simulate
        // what DataDictionary::new would do after fetching data

        let mock_response_data = json!({
            "title": "Marine Data Collection Schema",
            "fields": [
                {
                    "name": "  sample_id  ",         // Needs trimming
                    "title": "Sample\tID*",          // Needs control char normalization + asterisk preservation
                    "type": "string",
                    "description": "Unique sample identifier",
                    "constraints": {
                        "required": true
                    }
                },
                {
                    "name": "collection_date\n",     // Needs control char removal
                    "title": "Collection Date",
                    "type": "datetime",
                    "format": "%Y-%m-%d",
                    "description": "Date of sample collection"
                },
                {
                    "name": "temperature",
                    "title": "Temperature\r(°C)*",   // Control chars + special chars + asterisk
                    "type": "number",
                    "constraints": {
                        "minimum": -50.0,
                        "maximum": 100.0
                    }
                },
                {
                    "name": "location_notes\t",      // Trailing control char
                    "title": "Location Notes",
                    "type": "string",
                    "constraints": {
                        "maxLength": 500
                    }
                },
                {
                    "name": "quality_flag",          // No normalization needed
                    "title": "Quality Flag",
                    "type": "boolean"
                }
            ]
        });

        // Simulate what DataDictionary::new does: normalize during initialization
        let normalized_data =
            DataDictionary::normalize_field_data_for_tests(mock_response_data.clone()).unwrap();

        // Test that instance methods work with pre-normalized data
        let schema =
            DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data).unwrap();
        let title_mapping = DataDictionary::create_title_to_name_mapping(&normalized_data).unwrap();

        // Verify schema properties use normalized titles
        let properties = schema.get("properties").unwrap().as_object().unwrap();
        assert!(
            properties.contains_key("Sample ID*"),
            "Normalized title with preserved asterisk should exist"
        );
        assert!(
            properties.contains_key("Collection Date"),
            "Simple title should exist"
        );
        assert!(
            properties.contains_key("Temperature (°C)*"),
            "Complex title with special chars and asterisk should exist"
        );
        assert!(
            properties.contains_key("Location Notes"),
            "Simple title should exist"
        );
        assert!(
            properties.contains_key("Quality Flag"),
            "Simple title should exist"
        );

        // Verify control characters were normalized
        assert!(
            !properties.contains_key("Sample\tID*"),
            "Raw title with tab should not exist"
        );
        assert!(
            !properties.contains_key("Temperature\r(°C)*"),
            "Raw title with carriage return should not exist"
        );

        // Verify title-to-name mapping uses normalized data
        let expected_mapping: HashMap<String, String> = [
            ("Sample ID*".to_string(), "sample_id".to_string()), // Trimmed name, normalized title
            ("Collection Date".to_string(), "collection_date".to_string()), // Control char removed from name
            ("Temperature (°C)*".to_string(), "temperature".to_string()),   // Normalized title
            ("Location Notes".to_string(), "location_notes".to_string()), // Control char removed from name
            ("Quality Flag".to_string(), "quality_flag".to_string()),     // No changes needed
        ]
        .into_iter()
        .collect();

        assert_eq!(title_mapping, expected_mapping);

        // Verify required fields include asterisk-marked fields
        let required = schema.get("required").unwrap().as_array().unwrap();

        // Sample ID* (explicit required + asterisk) and Temperature (°C)* (asterisk)
        assert_eq!(required.len(), 2);
        assert!(required.contains(&json!("Sample ID*")));
        assert!(required.contains(&json!("Temperature (°C)*")));
    }

    #[test]
    fn test_normalization_with_duplicate_detection() {
        // Test that normalization works correctly with duplicate detection

        let data_with_normalized_duplicates = json!({
            "title": "Duplicate Detection Test",
            "fields": [
                {
                    "name": "field_name",
                    "title": "Field Title",
                    "type": "string"
                },
                {
                    "name": "  field_name  ",      // Duplicate after normalization
                    "title": "Field\tTitle",       // Duplicate after normalization
                    "type": "number"
                }
            ]
        });

        // Check_duplicates should work with raw data (it normalizes internally)
        let result = DataDictionary::check_duplicates(&data_with_normalized_duplicates);
        assert!(
            result.is_err(),
            "Should detect duplicates after normalization"
        );

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("field_name"),
            "Error should mention duplicate name"
        );
        assert!(
            error_message.contains("Field Title"),
            "Error should mention duplicate title"
        );
    }

    #[test]
    fn test_mixed_normalization_scenarios() {
        // Test various edge cases that might occur in real data

        let complex_data = json!({
            "title": "Complex Real-World Scenario",
            "fields": [
                {
                    "name": "id",
                    "title": "ID",
                    "type": "integer",
                    "constraints": {"required": true}
                },
                {
                    "name": "\r\n  whitespace_heavy  \t",
                    "title": "Heavy\nWhitespace\rField*",
                    "type": "string"
                },
                {
                    "name": "unicode_test",
                    "title": "Conductivity (μS/cm²)",  // Unicode preserved
                    "type": "number"
                },
                {
                    "name": "asterisk_name*",
                    "title": "Asterisk Title*",         // Both have asterisks
                    "type": "string"
                },
                {
                    "name": "no_title_field",
                    // No title provided
                    "type": "boolean"
                },
                {
                    "name": "empty_title_field",
                    "title": "",                        // Empty title
                    "type": "string"
                },
                {
                    "name": "spaces_only_title",
                    "title": "   \t   ",               // Only whitespace in title
                    "type": "string"
                }
            ]
        });

        let normalized_data = DataDictionary::normalize_field_data_for_tests(complex_data).unwrap();
        let schema =
            DataDictionary::convert_data_dictionary_to_json_schema(&normalized_data).unwrap();
        let mapping = DataDictionary::create_title_to_name_mapping(&normalized_data).unwrap();

        let properties = schema.get("properties").unwrap().as_object().unwrap();

        // Check that all expected normalized properties exist
        assert!(properties.contains_key("ID"));
        assert!(properties.contains_key("Heavy Whitespace Field*"));
        assert!(properties.contains_key("Conductivity (μS/cm²)"));
        assert!(properties.contains_key("Asterisk Title*"));
        assert!(properties.contains_key("no_title_field")); // Falls back to name
        assert!(properties.contains_key("")); // Empty title from empty/whitespace fields

        // We expect 6 properties because empty_title_field and spaces_only_title
        // both normalize to empty titles, causing a conflict (duplicate empty string key)
        assert_eq!(
            properties.len(),
            6,
            "Should have exactly 6 properties (empty title creates conflict)"
        );

        // Check mapping handles edge cases correctly
        assert_eq!(
            mapping.get("Heavy Whitespace Field*"),
            Some(&"whitespace_heavy".to_string())
        );
        assert_eq!(
            mapping.get("Conductivity (μS/cm²)"),
            Some(&"unicode_test".to_string())
        );
        assert_eq!(
            mapping.get("Asterisk Title*"),
            Some(&"asterisk_name*".to_string())
        );
        assert_eq!(
            mapping.get("no_title_field"),
            Some(&"no_title_field".to_string())
        );

        // For empty/whitespace titles, they normalize to empty and fall back to field name
        // Both empty_title_field and spaces_only_title have empty normalized titles,
        // so they should map their field names to themselves
        assert!(mapping.contains_key("empty_title_field") || mapping.contains_key(""));
        assert!(
            mapping.contains_key("spaces_only_title")
                || mapping.values().any(|v| v == "spaces_only_title")
        );

        // Check required fields (should include asterisk fields and explicit constraints)
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("ID"))); // Explicit constraint
        assert!(required.contains(&json!("Heavy Whitespace Field*"))); // Asterisk in title
        assert!(required.contains(&json!("Asterisk Title*"))); // Asterisk in both name and title
    }

    #[test]
    fn test_performance_optimization_verification() {
        // This test verifies that our optimization works as expected by ensuring
        // that normalization results are consistent regardless of how many times we call
        // the conversion functions (simulating that DataDictionary only normalizes once)

        let test_data = json!({
            "title": "Performance Test",
            "fields": [
                {"name": "\tfield1\n", "title": "Field\r1*", "type": "string"},
                {"name": "  field2  ", "title": "Field 2", "type": "number"},
                {"name": "field3", "type": "boolean"}  // No title
            ]
        });

        // Normalize once (simulating DataDictionary::new)
        let normalized_once =
            DataDictionary::normalize_field_data_for_tests(test_data.clone()).unwrap();

        // Generate schema and mapping multiple times from the same normalized data
        let schema1 =
            DataDictionary::convert_data_dictionary_to_json_schema(&normalized_once).unwrap();
        let schema2 =
            DataDictionary::convert_data_dictionary_to_json_schema(&normalized_once).unwrap();
        let mapping1 = DataDictionary::create_title_to_name_mapping(&normalized_once).unwrap();
        let mapping2 = DataDictionary::create_title_to_name_mapping(&normalized_once).unwrap();

        // All results should be identical (no additional normalization happening)
        assert_eq!(
            schema1, schema2,
            "Multiple schema generations should be identical"
        );
        assert_eq!(
            mapping1, mapping2,
            "Multiple mapping generations should be identical"
        );

        // Compare with fresh normalization to ensure consistency
        let normalized_fresh = DataDictionary::normalize_field_data_for_tests(test_data).unwrap();
        assert_eq!(
            normalized_once, normalized_fresh,
            "Normalization should be deterministic"
        );
    }
}
