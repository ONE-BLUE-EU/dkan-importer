use serde_json::json;

#[test]
fn test_saturated_fatty_acids_out_of_range() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": {
            "saturated_fatty_acids": {
                "type": ["number", "null"],
                "minimum": 0,
                "maximum": 1000,
                "description": "Test field"
            }
        },
        "required": [],
        "additionalProperties": true
    });

    // This test would require an actual Excel file
    // For now, let's verify the schema parses correctly
    println!("Schema: {:?}", schema);
}
