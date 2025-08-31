use dkan_importer::model::ExcelValidator;
use dkan_importer::utils::normalize_string;
use proptest::prelude::*;
use serde_json::json;

#[test]
fn test_normalize_string_basic() {
    assert_eq!(normalize_string("Simple Header"), "Simple Header");
}

#[test]
fn test_normalize_string_multiple_spaces() {
    // Test that multiple consecutive spaces are collapsed into single spaces
    assert_eq!(
        normalize_string("Header  with   multiple    spaces"),
        "Header with multiple spaces"
    );

    // Test leading and trailing spaces are trimmed
    assert_eq!(
        normalize_string("   Leading and trailing spaces   "),
        "Leading and trailing spaces"
    );

    // Test mixture of spaces, tabs, and newlines - all normalized to single spaces
    assert_eq!(
        normalize_string("Mixed   \t\n  whitespace   characters"),
        "Mixed whitespace characters"
    );

    // Test extreme case with many spaces
    assert_eq!(
        normalize_string("Too      many          spaces      here"),
        "Too many spaces here"
    );

    // Test with asterisks and multiple spaces
    assert_eq!(
        normalize_string("Required   Field*   with    spaces*"),
        "Required Field* with spaces*"
    );

    // Test empty string with spaces
    assert_eq!(normalize_string("   "), "");

    // Test single space (should remain unchanged)
    assert_eq!(normalize_string(" "), "");
}

#[test]
fn test_normalize_string_preserves_asterisks() {
    assert_eq!(normalize_string("Required Field*"), "Required Field*");

    assert_eq!(
        normalize_string("Multiple Asterisks***"),
        "Multiple Asterisks***"
    );
}

#[test]
fn test_normalize_string_with_newlines() {
    assert_eq!(
        normalize_string("Remark 1\nAnalytical Partner"),
        "Remark 1 Analytical Partner"
    );

    assert_eq!(
        normalize_string("First Line\nSecond Line\nThird Line"),
        "First Line Second Line Third Line"
    );
}

#[test]
fn test_normalize_string_with_control_characters() {
    assert_eq!(normalize_string("Header\tWith\tTabs"), "Header With Tabs");

    assert_eq!(
        normalize_string("Header\rWith\rCarriage"),
        "Header With Carriage"
    );
}

#[test]
fn test_normalize_string_complex() {
    assert_eq!(
        normalize_string("  \tComplicated *\nWith Everything\r  "),
        "Complicated * With Everything"
    );
}

#[test]
fn test_normalize_string_edge_cases() {
    // Empty string
    assert_eq!(normalize_string(""), "");

    // Only asterisks - now preserved
    assert_eq!(normalize_string("***"), "***");

    // Only whitespace and control characters
    assert_eq!(normalize_string("  \t\r\n  "), "");

    // Just newline
    assert_eq!(normalize_string("\n"), "");
}

#[test]
fn test_normalize_string_preserves_internal_asterisks() {
    assert_eq!(
        normalize_string("Field * With Internal"),
        "Field * With Internal"
    );
}

#[test]
fn test_normalize_string_real_world_examples() {
    // From the original error message - asterisks now preserved
    assert_eq!(
        normalize_string("Date of sampling start*"),
        "Date of sampling start*"
    );

    assert_eq!(normalize_string("Name of sea*"), "Name of sea*");

    assert_eq!(
        normalize_string("Remark 1\nAnalytical Partner"),
        "Remark 1 Analytical Partner"
    );

    assert_eq!(
        normalize_string("Remark 2\nDissolved Oxygen Concentration (mg/kg)"),
        "Remark 2 Dissolved Oxygen Concentration (mg/kg)"
    );
}

// Property-based tests using proptest
proptest! {
     #![proptest_config(ProptestConfig {
        cases: 10000, ..ProptestConfig::default()
        })]

    #[test]
    fn test_normalize_string_preserves_trailing_asterisks(
        base in "[a-zA-Z0-9 ]+",
        asterisks in "\\*+"
    ) {
        let input = format!("{}{}", base, asterisks);
        let result = normalize_string(&input);

        // Result should preserve asterisks
        prop_assert!(result.ends_with(&asterisks));
        // Result should start with the normalized base (whitespace normalized)
        if !base.trim().is_empty() {
            let normalized_base = base.split_whitespace().collect::<Vec<&str>>().join(" ");
            prop_assert!(result.starts_with(&normalized_base));
        }
    }

    #[test]
    fn test_normalize_string_preserves_all_text_with_spaces(
        first_line in "[a-zA-Z0-9 *]+",
        second_line in "[a-zA-Z0-9 *]+"
    ) {
        let input = format!("{}\n{}", first_line, second_line);
        let result = normalize_string(&input);

        // Result should contain both lines separated by space, with normalized whitespace
        let normalized_first = first_line.split_whitespace().collect::<Vec<&str>>().join(" ");
        let normalized_second = second_line.split_whitespace().collect::<Vec<&str>>().join(" ");

        let expected = if normalized_first.is_empty() && !normalized_second.is_empty() {
            normalized_second
        } else if !normalized_first.is_empty() && normalized_second.is_empty() {
            normalized_first
        } else if !normalized_first.is_empty() && !normalized_second.is_empty() {
            format!("{} {}", normalized_first, normalized_second)
        } else {
            String::new() // Both are empty
        };

        prop_assert_eq!(result, expected);
    }

    #[test]
    fn test_normalize_string_replaces_control_chars_with_spaces(
        prefix in "[a-zA-Z0-9 ]*",
        suffix in "[a-zA-Z0-9 ]*"
    ) {
        // Insert various control characters
        let control_chars = ['\t', '\r', '\x0B', '\x0C']; // tab, carriage return, vertical tab, form feed

        for &ctrl in &control_chars {
            let input = format!("{}{}{}", prefix, ctrl, suffix);
            let result = normalize_string(&input);

            // Result should not contain the control character
            prop_assert!(!result.contains(ctrl));

            // If both prefix and suffix have content, result should contain both with normalized whitespace
            if !prefix.trim().is_empty() && !suffix.trim().is_empty() {
                let normalized_prefix = prefix.split_whitespace().collect::<Vec<&str>>().join(" ");
                let normalized_suffix = suffix.split_whitespace().collect::<Vec<&str>>().join(" ");
                let expected = format!("{} {}", normalized_prefix, normalized_suffix);
                prop_assert_eq!(result, expected);
            }
        }
    }

    #[test]
    fn test_normalize_string_idempotent(
        input in "[a-zA-Z0-9 ]+[*]*"
    ) {
        let first_pass = normalize_string(&input);
        let second_pass = normalize_string(&first_pass);

        // Normalizing twice should give same result
        prop_assert_eq!(first_pass, second_pass);
    }
}

#[test]
fn test_integration_with_schema_validation() -> anyhow::Result<()> {
    // Create a simple schema with full titles (as they would be in DKAN titles)
    let schema = json!({
        "type": "object",
        "properties": {
            "Date of sampling start*": {"type": "string"},
            "Name of sea*": {"type": "string"},
            "Remark 1 Analytical Partner": {"type": "string"}
        },
        "required": ["Date of sampling start*", "Name of sea*"],
        "additionalProperties": false
    });

    let validator = ExcelValidator::new(&schema, "test_errors.log")?;

    // Test that normalized headers would create valid JSON
    let normalized_headers = vec![
        normalize_string("Date of sampling start*"),
        normalize_string("Name of sea*"),
        normalize_string("Remark 1\nAnalytical Partner"),
    ];

    // Create JSON object using normalized headers
    let mut test_data = serde_json::Map::new();
    test_data.insert(normalized_headers[0].clone(), json!("2024-01-15"));
    test_data.insert(normalized_headers[1].clone(), json!("Mediterranean"));
    test_data.insert(
        normalized_headers[2].clone(),
        json!("Test remark with analytical partner info"),
    );

    let test_json = json!(test_data);

    // Should validate successfully
    assert!(validator.validator.is_valid(&test_json));

    Ok(())
}
