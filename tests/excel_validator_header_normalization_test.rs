use dkan_importer::model::ExcelValidator;
use proptest::prelude::*;
use serde_json::json;

#[test]
fn test_normalize_excel_header_basic() {
    assert_eq!(
        ExcelValidator::normalize_excel_header("Simple Header".to_string()),
        "Simple Header"
    );
}

#[test]
fn test_normalize_excel_header_multiple_spaces() {
    // Test that multiple consecutive spaces are collapsed into single spaces
    assert_eq!(
        ExcelValidator::normalize_excel_header("Header  with   multiple    spaces".to_string()),
        "Header with multiple spaces"
    );

    // Test leading and trailing spaces are trimmed
    assert_eq!(
        ExcelValidator::normalize_excel_header("   Leading and trailing spaces   ".to_string()),
        "Leading and trailing spaces"
    );

    // Test mixture of spaces, tabs, and newlines - all normalized to single spaces
    assert_eq!(
        ExcelValidator::normalize_excel_header("Mixed   \t\n  whitespace   characters".to_string()),
        "Mixed whitespace characters"
    );

    // Test extreme case with many spaces
    assert_eq!(
        ExcelValidator::normalize_excel_header(
            "Too      many          spaces      here".to_string()
        ),
        "Too many spaces here"
    );

    // Test with asterisks and multiple spaces
    assert_eq!(
        ExcelValidator::normalize_excel_header("Required   Field*   with    spaces*".to_string()),
        "Required Field* with spaces*"
    );

    // Test empty string with spaces
    assert_eq!(
        ExcelValidator::normalize_excel_header("   ".to_string()),
        ""
    );

    // Test single space (should remain unchanged)
    assert_eq!(ExcelValidator::normalize_excel_header(" ".to_string()), "");
}

#[test]
fn test_normalize_excel_header_preserves_asterisks() {
    assert_eq!(
        ExcelValidator::normalize_excel_header("Required Field*".to_string()),
        "Required Field*"
    );

    assert_eq!(
        ExcelValidator::normalize_excel_header("Multiple Asterisks***".to_string()),
        "Multiple Asterisks***"
    );
}

#[test]
fn test_normalize_excel_header_with_newlines() {
    assert_eq!(
        ExcelValidator::normalize_excel_header("Remark 1\nAnalytical Partner".to_string()),
        "Remark 1 Analytical Partner"
    );

    assert_eq!(
        ExcelValidator::normalize_excel_header("First Line\nSecond Line\nThird Line".to_string()),
        "First Line Second Line Third Line"
    );
}

#[test]
fn test_normalize_excel_header_with_control_characters() {
    assert_eq!(
        ExcelValidator::normalize_excel_header("Header\tWith\tTabs".to_string()),
        "Header With Tabs"
    );

    assert_eq!(
        ExcelValidator::normalize_excel_header("Header\rWith\rCarriage".to_string()),
        "Header With Carriage"
    );
}

#[test]
fn test_normalize_excel_header_complex() {
    assert_eq!(
        ExcelValidator::normalize_excel_header(
            "  \tComplicated *\nWith Everything\r  ".to_string()
        ),
        "Complicated * With Everything"
    );
}

#[test]
fn test_normalize_excel_header_edge_cases() {
    // Empty string
    assert_eq!(ExcelValidator::normalize_excel_header("".to_string()), "");

    // Only asterisks - now preserved
    assert_eq!(
        ExcelValidator::normalize_excel_header("***".to_string()),
        "***"
    );

    // Only whitespace and control characters
    assert_eq!(
        ExcelValidator::normalize_excel_header("  \t\r\n  ".to_string()),
        ""
    );

    // Just newline
    assert_eq!(ExcelValidator::normalize_excel_header("\n".to_string()), "");
}

#[test]
fn test_normalize_excel_header_preserves_internal_asterisks() {
    assert_eq!(
        ExcelValidator::normalize_excel_header("Field * With Internal".to_string()),
        "Field * With Internal"
    );
}

#[test]
fn test_normalize_excel_header_real_world_examples() {
    // From the original error message - asterisks now preserved
    assert_eq!(
        ExcelValidator::normalize_excel_header("Date of sampling start*".to_string()),
        "Date of sampling start*"
    );

    assert_eq!(
        ExcelValidator::normalize_excel_header("Name of sea*".to_string()),
        "Name of sea*"
    );

    assert_eq!(
        ExcelValidator::normalize_excel_header("Remark 1\nAnalytical Partner".to_string()),
        "Remark 1 Analytical Partner"
    );

    assert_eq!(
        ExcelValidator::normalize_excel_header(
            "Remark 2\nDissolved Oxygen Concentration (mg/kg)".to_string()
        ),
        "Remark 2 Dissolved Oxygen Concentration (mg/kg)"
    );
}

// Property-based tests using proptest
proptest! {
    #[test]
    fn test_normalize_excel_header_preserves_trailing_asterisks(
        base in "[a-zA-Z0-9 ]+",
        asterisks in "\\*+"
    ) {
        let input = format!("{}{}", base, asterisks);
        let result = ExcelValidator::normalize_excel_header(input);

        // Result should preserve asterisks
        prop_assert!(result.ends_with(&asterisks));
        // Result should start with the normalized base (whitespace normalized)
        if !base.trim().is_empty() {
            let normalized_base = base.split_whitespace().collect::<Vec<&str>>().join(" ");
            prop_assert!(result.starts_with(&normalized_base));
        }
    }

    #[test]
    fn test_normalize_excel_header_preserves_all_text_with_spaces(
        first_line in "[a-zA-Z0-9 *]+",
        second_line in "[a-zA-Z0-9 *]+"
    ) {
        let input = format!("{}\n{}", first_line, second_line);
        let result = ExcelValidator::normalize_excel_header(input);

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
    fn test_normalize_excel_header_replaces_control_chars_with_spaces(
        prefix in "[a-zA-Z0-9 ]*",
        suffix in "[a-zA-Z0-9 ]*"
    ) {
        // Insert various control characters
        let control_chars = ['\t', '\r', '\x0B', '\x0C']; // tab, carriage return, vertical tab, form feed

        for &ctrl in &control_chars {
            let input = format!("{}{}{}", prefix, ctrl, suffix);
            let result = ExcelValidator::normalize_excel_header(input);

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
    fn test_normalize_excel_header_idempotent(
        input in "[a-zA-Z0-9 ]+[*]*"
    ) {
        let first_pass = ExcelValidator::normalize_excel_header(input);
        let second_pass = ExcelValidator::normalize_excel_header(first_pass.clone());

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
        ExcelValidator::normalize_excel_header("Date of sampling start*".to_string()),
        ExcelValidator::normalize_excel_header("Name of sea*".to_string()),
        ExcelValidator::normalize_excel_header("Remark 1\nAnalytical Partner".to_string()),
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
