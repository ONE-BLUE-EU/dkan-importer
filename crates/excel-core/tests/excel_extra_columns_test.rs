//! Moving the sheet's columns that are not data dictionary fields into one dictionary field as a
//! JSON object, through the real file pipeline: write an .xlsx, build the validator from it,
//! validate, and export the CSV.

mod common;

use common::Cell::{self, Empty, Number, Text};
use common::{create_simple_mapping, write_test_workbook};
use excel_core::serde_json::json;
use excel_core::{ExcelValidatorBuilder, Unpivot};

/// Dictionary fields: a tag, two text remarks (one of them not in the sheet), and a number.
fn schema() -> excel_core::serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "Sample Tag": { "type": "string" },
            "Remark 1": { "type": ["string", "null"] },
            "Remark 2": { "type": ["string", "null"] },
            "Depth": { "type": ["number", "null"] }
        },
        "required": ["Sample Tag"],
        "additionalProperties": false
    })
}

/// Two extra columns, C and E, around a dictionary column, in an order that is not alphabetical.
fn sheet() -> Vec<Vec<Cell>> {
    vec![
        vec![
            Text("Sample Tag"),
            Text("Remark 1"),
            Text("Slice num"),
            Text("Depth"),
            Text("DNA (Yes/No)"),
        ],
        vec![Text("ST01-a"), Empty, Number(1.0), Number(5.0), Text("yes")],
        vec![
            Text("ST01-b"),
            Text("Super dirty"),
            Number(30.5),
            Number(6.0),
            Text("no"),
        ],
        vec![Text("ST01-c"), Text("Flow: 1-2"), Empty, Number(7.0), Empty],
        vec![Text("ST01-d"), Empty, Empty, Number(8.0), Empty],
    ]
}

#[derive(Debug)]
struct Moved {
    summary: String,
    csv_lines: Vec<String>,
}

/// Build with `extra_columns_to(field)`, validate and export `rows` (the first row being the
/// headers).
fn run_with(
    name: &str,
    rows: &[Vec<Cell>],
    schema: excel_core::serde_json::Value,
    field: &str,
) -> Result<Moved, String> {
    let xlsx = write_test_workbook(name, rows);
    let csv = xlsx.with_extension("csv");

    let result = (|| {
        let mut validator = ExcelValidatorBuilder::new(xlsx.to_str().unwrap(), "Sheet1", schema)
            .extra_columns_to(field)
            .build()
            .map_err(|e| format!("{e:#}"))?;
        let summary = validator
            .extra_columns_summary()
            .ok_or("moving extra columns must give a summary")?
            .to_string();
        validator.validate_excel().map_err(|e| {
            let details: Vec<String> = validator
                .validation_reports
                .iter()
                .flat_map(|r| r.errors.iter().map(|e| e.to_string()))
                .collect();
            format!("{e}: {}", details.join(" | "))
        })?;
        validator
            .export_to_csv(
                csv.to_str().unwrap(),
                create_simple_mapping(vec!["Sample Tag", "Remark 1", "Remark 2", "Depth"]),
            )
            .map_err(|e| e.to_string())?;
        let csv_lines = std::fs::read_to_string(&csv)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        Ok(Moved { summary, csv_lines })
    })();

    let _ = std::fs::remove_file(&xlsx);
    let _ = std::fs::remove_file(&csv);
    result
}

fn run(name: &str, rows: &[Vec<Cell>], field: &str) -> Result<Moved, String> {
    run_with(name, rows, schema(), field)
}

#[test]
fn extra_columns_become_a_json_object_in_the_target_field() {
    let moved = run("extra-json", &sheet(), "Remark 1").unwrap();

    // Keys in sheet order; numbers stay numbers, text stays text, blank cells are left out. Text
    // already in the target is kept under its own header, first. Rows without extra data keep the
    // target as it was, text or empty.
    let expected = vec![
        "Sample Tag,Remark 1,Depth",
        r#"ST01-a,"{""Slice num"":1,""DNA (Yes/No)"":""yes""}",5"#,
        r#"ST01-b,"{""Remark 1"":""Super dirty"",""Slice num"":30.5,""DNA (Yes/No)"":""no""}",6"#,
        "ST01-c,Flow: 1-2,7",
        "ST01-d,,8",
    ];
    assert_eq!(moved.csv_lines, expected);
}

#[test]
fn the_summary_names_the_moved_columns_and_the_rows_with_extra_data() {
    let moved = run("extra-summary", &sheet(), "Remark 1").unwrap();

    let expected = "Moving 2 extra column(s) into Remark 1: column C \"Slice num\", column E \"DNA (Yes/No)\"; 2 row(s) have extra data.";
    assert_eq!(moved.summary, expected);
}

#[test]
fn a_target_that_is_not_a_sheet_column_is_added_to_the_export() {
    let moved = run("extra-new-column", &sheet(), "Remark 2").unwrap();

    let expected = vec![
        "Sample Tag,Remark 1,Depth,Remark 2",
        r#"ST01-a,,5,"{""Slice num"":1,""DNA (Yes/No)"":""yes""}""#,
        r#"ST01-b,Super dirty,6,"{""Slice num"":30.5,""DNA (Yes/No)"":""no""}""#,
        "ST01-c,Flow: 1-2,7,",
        "ST01-d,,8,",
    ];
    assert_eq!(moved.csv_lines, expected);
}

#[test]
fn a_sheet_without_extra_columns_is_left_as_it_is() {
    let rows = vec![
        vec![Text("Sample Tag"), Text("Remark 1")],
        vec![Text("ST01-a"), Text("Flow: 1-2")],
    ];

    let moved = run("extra-none", &rows, "Remark 1").unwrap();

    let expected_summary = "No extra columns to move into Remark 1.";
    assert_eq!(moved.summary, expected_summary);
    let expected_csv = vec!["Sample Tag,Remark 1", "ST01-a,Flow: 1-2"];
    assert_eq!(moved.csv_lines, expected_csv);
}

/// The JSON goes through the target field's schema like any other value.
#[test]
fn json_longer_than_the_target_allows_is_a_validation_error_at_its_row() {
    let mut schema = schema();
    schema["properties"]["Remark 1"]["maxLength"] = json!(30);

    let err = run_with("extra-max-length", &sheet(), schema, "Remark 1")
        .expect_err("a too long JSON must be refused");

    assert!(err.contains("row[2]./Remark 1"), "{err}");
    assert!(err.contains("row[3]./Remark 1"), "{err}");
    assert!(!err.contains("row[4]"), "{err}");
}

#[test]
fn a_target_that_is_not_a_dictionary_field_is_refused_with_the_candidates() {
    let err = run("extra-unknown-target", &sheet(), "Remark 9")
        .expect_err("an unknown target must be refused");

    assert!(err.contains("\"Remark 9\""), "{err}");
    assert!(err.contains("Candidates: Remark 1, Remark 2"), "{err}");
}

#[test]
fn a_target_that_does_not_take_text_is_refused_with_the_candidates() {
    let err =
        run("extra-number-target", &sheet(), "Depth").expect_err("a number target must be refused");

    assert!(err.contains("\"Depth\""), "{err}");
    assert!(err.contains("Candidates: Remark 1, Remark 2"), "{err}");
}

#[test]
fn data_in_a_column_without_a_header_is_still_refused() {
    let rows = vec![
        vec![Text("Sample Tag"), Text("Slice num"), Empty],
        vec![Text("ST01-a"), Number(1.0), Text("stray")],
    ];

    let err = run("extra-unnamed", &rows, "Remark 1")
        .expect_err("a column without a header must be refused");

    assert!(
        err.contains("column C (first value at row 2: \"stray\")"),
        "{err}"
    );
}

#[test]
fn moving_extra_columns_and_unpivoting_together_is_refused() {
    let xlsx = write_test_workbook("extra-and-unpivot", &sheet());

    let result = ExcelValidatorBuilder::new(xlsx.to_str().unwrap(), "Sheet1", schema())
        .extra_columns_to("Remark 1")
        .unpivot(Unpivot {
            headers_column: "Remark 2".to_string(),
            values_column: "Depth".to_string(),
            values_markers: Vec::new(),
            marker_limits: Vec::new(),
        })
        .build();
    let _ = std::fs::remove_file(&xlsx);

    let err = format!("{:#}", result.err().expect("both must be refused"));
    assert!(err.contains("not both"), "{err}");
}

/// Without the option, the same sheet still fails on its extra columns.
#[test]
fn without_the_option_extra_columns_are_still_refused() {
    let xlsx = write_test_workbook("extra-off", &sheet());

    let mut validator = ExcelValidatorBuilder::new(xlsx.to_str().unwrap(), "Sheet1", schema())
        .build()
        .unwrap();
    let result = validator.validate_excel();
    let _ = std::fs::remove_file(&xlsx);

    assert!(result.is_err());
    assert!(validator.extra_columns_summary().is_none());
    let errors: Vec<String> = validator
        .validation_reports
        .iter()
        .flat_map(|r| r.errors.iter().map(|e| e.to_string()))
        .collect();
    let expected = "extra columns not found in the provided JSON Schema";
    assert!(errors.iter().any(|e| e.contains(expected)), "{errors:?}");
}
