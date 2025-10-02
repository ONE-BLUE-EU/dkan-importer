//! Tests for Excel validation with asterisk-containing field names
//! Tests that ExcelValidator correctly handles property names with asterisks
//! and validates required fields appropriately

use importer_lib::ExcelValidator;
use serde_json::json;

#[test]
fn test_excel_matching_with_preserved_asterisks() {
    // Test that Excel column matching works when property names have asterisks
    // This test directly uses JSON Schema without DKAN DataDictionary conversion
    let json_schema = json!({
        "type": "object",
        "properties": {
            "Sample ID": {              // Property name from title (no asterisk)
                "type": "string"
            },
            "Temperature*": {           // Property name from title with asterisk
                "type": "number"
            },
            "Notes": {                  // Optional field
                "type": "string"
            }
        },
        "required": ["Sample ID", "Temperature*"],  // Both are required
        "additionalProperties": false
    });

    let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();

    // Test Excel data that matches the property names (including asterisks)
    let excel_data = json!({
        "Sample ID": "SAMPLE_001",        // Required field
        "Temperature*": 22.5,             // Required field with asterisk in name
        "Notes": ""                       // Optional string field can be empty string
    });

    let is_valid = validator.validator.is_valid(&excel_data);
    assert!(
        is_valid,
        "Excel data should validate when all required fields are present"
    );

    // Test missing required field with asterisk in property name
    let invalid_data = json!({
        // "Sample ID": "SAMPLE_002",     // Missing required field
        "Temperature*": 20.0,
        "Notes": "test"
    });

    assert!(
        !validator.validator.is_valid(&invalid_data),
        "Should fail when required field 'Sample ID' is missing"
    );

    // Test missing required field that has asterisk in its name
    let invalid_data2 = json!({
        "Sample ID": "SAMPLE_003",
        // "Temperature*": 18.5,         // Missing required field with asterisk
        "Notes": "test"
    });

    assert!(
        !validator.validator.is_valid(&invalid_data2),
        "Should fail when required field 'Temperature*' is missing"
    );

    // Test with null value for required field
    let invalid_data3 = json!({
        "Sample ID": null,                // Required field cannot be null
        "Temperature*": 25.0,
        "Notes": "test"
    });

    assert!(
        !validator.validator.is_valid(&invalid_data3),
        "Should fail when required field has null value"
    );

    // Test with extra properties (should fail with additionalProperties: false)
    let invalid_data4 = json!({
        "Sample ID": "SAMPLE_004",
        "Temperature*": 22.0,
        "Notes": "test",
        "ExtraField": "unexpected"        // Extra property not in schema
    });

    assert!(
        !validator.validator.is_valid(&invalid_data4),
        "Should fail when additional properties are present"
    );
}

#[test]
fn test_real_world_scenario_with_preserved_asterisks() {
    // Realistic scenario with mixed asterisk patterns in property names
    // Simulates what would result from DKAN schema conversion where:
    // - Properties are named after titles
    // - Asterisks are preserved in property names
    // - Required fields are those with asterisks in original name or title
    let json_schema = json!({
        "type": "object",
        "title": "Marine Data Collection with Mixed Asterisks",
        "properties": {
            "Sample Identifier": {           // Required (had name asterisk)
                "type": "string",
                "title": "Sample Identifier"
            },
            "Collection Date*": {            // Required (has title asterisk)
                "type": "string",
                "title": "Collection Date*"
            },
            "Latitude*": {                   // Required (both name and title had asterisk)
                "type": "number",
                "title": "Latitude*",
                "minimum": -90.0,
                "maximum": 90.0
            },
            "Longitude": {                   // Required (had name asterisk, title without)
                "type": "number",
                "title": "Longitude",
                "minimum": -180.0,
                "maximum": 180.0
            },
            "Water Temperature (°C)": {      // Optional (no asterisks)
                "type": ["number", "null"],
                "title": "Water Temperature (°C)"
            },
            "Salinity (ppt)": {              // Optional (no asterisks)
                "type": ["number", "null"],
                "title": "Salinity (ppt)"
            }
        },
        "required": ["Sample Identifier", "Collection Date*", "Latitude*", "Longitude"],
        "additionalProperties": false
    });

    // Verify required fields
    let required = json_schema.get("required").unwrap().as_array().unwrap();
    assert_eq!(required.len(), 4);
    assert!(required.contains(&json!("Sample Identifier"))); // Name asterisk
    assert!(required.contains(&json!("Collection Date*"))); // Title asterisk
    assert!(required.contains(&json!("Latitude*"))); // Both name and title asterisks
    assert!(required.contains(&json!("Longitude"))); // Name asterisk only

    // Verify all titles are preserved (properties are named after titles)
    let properties = json_schema.get("properties").unwrap().as_object().unwrap();
    assert_eq!(
        properties["Sample Identifier"]["title"],
        json!("Sample Identifier")
    );
    assert_eq!(
        properties["Collection Date*"]["title"],
        json!("Collection Date*")
    );
    assert_eq!(properties["Latitude*"]["title"], json!("Latitude*"));
    assert_eq!(properties["Longitude"]["title"], json!("Longitude"));
    assert_eq!(
        properties["Water Temperature (°C)"]["title"],
        json!("Water Temperature (°C)")
    );
    assert_eq!(
        properties["Salinity (ppt)"]["title"],
        json!("Salinity (ppt)")
    );

    // Verify type handling (using title-based property names)
    assert_eq!(properties["Sample Identifier"]["type"], json!("string")); // Mandatory = simple type
    assert_eq!(properties["Collection Date*"]["type"], json!("string")); // Mandatory datetime = string
    assert_eq!(properties["Latitude*"]["type"], json!("number")); // Mandatory = simple type
    assert_eq!(properties["Longitude"]["type"], json!("number")); // Mandatory = simple type
    assert_eq!(
        properties["Water Temperature (°C)"]["type"],
        json!(["number", "null"])
    ); // Optional = union type
    assert_eq!(
        properties["Salinity (ppt)"]["type"],
        json!(["number", "null"])
    ); // Optional = union type

    // Test validation with realistic data
    let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
    let valid_data = json!({
        "Sample Identifier": "MARINE_001",         // Required field
        "Collection Date*": "2024-01-15T10:30:00Z", // Required field with asterisk in title
        "Latitude*": 45.123,                      // Required field with asterisk
        "Longitude": 12.456,                       // Required field
        "Water Temperature (°C)": null,            // Optional field can be null
        "Salinity (ppt)": 35.2                     // Optional field with value
    });

    assert!(
        validator.validator.is_valid(&valid_data),
        "Realistic data should validate correctly"
    );

    // Test missing required field
    let invalid_data = json!({
        // Missing "Sample Identifier"
        "Collection Date*": "2024-01-15T10:30:00Z",
        "Latitude*": 45.123,
        "Longitude": 12.456,
        "Water Temperature (°C)": null,
        "Salinity (ppt)": 35.2
    });

    assert!(
        !validator.validator.is_valid(&invalid_data),
        "Should fail when required field is missing"
    );

    // Test with invalid number range
    let invalid_data2 = json!({
        "Sample Identifier": "MARINE_002",
        "Collection Date*": "2024-01-15T10:30:00Z",
        "Latitude*": 95.0,  // Out of range (> 90.0)
        "Longitude": 12.456,
        "Water Temperature (°C)": null,
        "Salinity (ppt)": 35.2
    });

    assert!(
        !validator.validator.is_valid(&invalid_data2),
        "Should fail when number is out of range"
    );

    // Test all optional fields can be null
    let valid_with_nulls = json!({
        "Sample Identifier": "MARINE_003",
        "Collection Date*": "2024-01-15T10:30:00Z",
        "Latitude*": 45.123,
        "Longitude": 12.456,
        "Water Temperature (°C)": null,
        "Salinity (ppt)": null
    });

    assert!(
        validator.validator.is_valid(&valid_with_nulls),
        "All optional fields should be allowed to be null"
    );
}
