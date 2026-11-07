//! End-to-end tests through the real file pipeline: build an .xlsx in memory, build the validator
//! from it, validate, and export the CSV that would be uploaded to DKAN. Nothing is written to
//! disk.

mod common;

use common::Cell::{Empty, Error, Number, Text};
use common::{create_simple_mapping, test_workbook};
use excel_core::ExcelValidatorBuilder;
use excel_core::serde_json::json;

fn sample_schema() -> excel_core::serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "Sample ID": { "type": "string" },
            "Depth": { "type": ["number", "null"] },
            "Count": { "type": ["integer", "null"] },
            "Notes": { "type": ["string", "null"] },
            "Date": { "type": ["string", "null"], "format": "date" }
        },
        "required": ["Sample ID"],
        "additionalProperties": false
    })
}

fn headers() -> Vec<common::Cell> {
    vec![
        Text("Sample ID"),
        Text("Depth"),
        Text("Count"),
        Text("Notes"),
        Text("Date"),
    ]
}

/// Validate and export `rows` (without headers); return the CSV lines, or the validation error.
fn run_pipeline(rows: Vec<Vec<common::Cell>>) -> Result<Vec<String>, String> {
    let mut all_rows = vec![headers()];
    all_rows.extend(rows);
    run_sheet(&all_rows)
}

/// Validate and export `all_rows` (the first row being the headers); return the CSV lines, or the
/// error.
fn run_sheet(all_rows: &[Vec<common::Cell>]) -> Result<Vec<String>, String> {
    run_sheet_with_error_cells(all_rows).map(|(_, lines)| lines)
}

/// [`run_sheet`], also returning what the validator reports about the Excel error cells it
/// emptied.
fn run_sheet_with_error_cells(
    all_rows: &[Vec<common::Cell>],
) -> Result<(Option<String>, Vec<String>), String> {
    let mut validator =
        ExcelValidatorBuilder::new(test_workbook(all_rows), "Sheet1", sample_schema())
            .build()
            .map_err(|e| format!("{e:#}"))?;
    let error_cells = validator.error_cells_summary().map(String::from);
    validator.validate_excel().map_err(|e| {
        let details: Vec<String> = validator
            .validation_reports
            .iter()
            .flat_map(|r| r.errors.iter().map(|e| e.to_string()))
            .collect();
        format!("{e}: {}", details.join(" | "))
    })?;
    let csv = validator
        .to_csv(&create_simple_mapping(vec![
            "Sample ID",
            "Depth",
            "Count",
            "Notes",
            "Date",
        ]))
        .map_err(|e| e.to_string())?;
    let lines = String::from_utf8(csv)
        .map_err(|e| e.to_string())?
        .lines()
        .map(String::from)
        .collect();
    Ok((error_cells, lines))
}

/// DKAN infers column types from the CSV, so empty numeric cells are deliberately written as
/// numeric placeholders. This test pins that decision; the row with values shows the same columns
/// carry real data when it is present.
#[test]
fn empty_numeric_cells_are_exported_as_dkan_type_placeholders() {
    let lines = run_pipeline(vec![
        vec![Text("S1"), Empty, Empty, Empty, Empty],
        vec![
            Text("S2"),
            Number(1.5),
            Number(3.0),
            Text("ok"),
            Text("2024-01-02"),
        ],
    ])
    .unwrap();

    assert_eq!(lines[0], "Sample ID,Depth,Count,Notes,Date");
    assert_eq!(lines[1], "S1,000000000000.000000,0,,");
    assert_eq!(lines[2], "S2,1.5,3,ok,2024-01-02");
}

#[test]
fn slash_dates_are_read_day_first() {
    let lines = run_pipeline(vec![vec![
        Text("S1"),
        Empty,
        Empty,
        Empty,
        Text("03/04/2024"),
    ]])
    .unwrap();

    assert_eq!(lines[1], "S1,000000000000.000000,0,,2024-04-03");
}

#[test]
fn month_first_date_is_a_validation_error() {
    let err = run_pipeline(vec![vec![
        Text("S1"),
        Empty,
        Empty,
        Empty,
        Text("08/28/2024"),
    ]])
    .unwrap_err();

    assert!(err.contains("'08/28/2024' is not a valid date"), "{err}");
}

#[test]
fn text_that_is_not_a_date_in_a_date_column_is_a_validation_error() {
    let err = run_pipeline(vec![vec![Text("S1"), Empty, Empty, Empty, Text("n/a")]]).unwrap_err();

    assert!(err.contains("'n/a' is not a valid date"), "{err}");
}

/// An error value is what a formula gives when its calculation fails (`#VALUE!` from text times a
/// number), so it carries no data: the cell is uploaded as if it were blank, and listed.
#[test]
fn excel_error_cells_are_uploaded_as_blank_cells_and_listed() {
    let mut rows = vec![headers()];
    rows.extend([
        vec![Text("S1"), Number(1.0), Empty, Text("ok"), Empty],
        vec![
            Text("S2"),
            Error("#N/A"),
            Error("#DIV/0!"),
            Error("#VALUE!"),
            Empty,
        ],
    ]);

    let (error_cells, lines) = run_sheet_with_error_cells(&rows).unwrap();

    let expected = vec![
        "Sample ID,Depth,Count,Notes,Date",
        "S1,1,0,ok,",
        "S2,000000000000.000000,0,,",
    ];
    assert_eq!(lines, expected);
    let expected = "Emptied 3 Excel error value(s), uploading them as blank cells:
  row 3, column B \"Depth\": #N/A
  row 3, column C \"Count\": #DIV/0!
  row 3, column D \"Notes\": #VALUE!";
    assert_eq!(error_cells.as_deref(), Some(expected));
}

#[test]
fn without_excel_error_cells_nothing_is_reported_about_them() {
    let mut rows = vec![headers()];
    rows.push(vec![Text("S1"), Number(1.0), Empty, Text("ok"), Empty]);

    let (error_cells, _) = run_sheet_with_error_cells(&rows).unwrap();

    assert_eq!(error_cells, None);
}

/// An error value without a header is no data either, so it is not refused as data without a
/// header.
#[test]
fn an_excel_error_cell_in_a_column_without_a_header_is_emptied_not_refused() {
    let mut rows = vec![headers()];
    rows.push(vec![
        Text("S1"),
        Number(1.0),
        Empty,
        Empty,
        Empty,
        Error("#VALUE!"),
    ]);

    let (error_cells, lines) = run_sheet_with_error_cells(&rows).unwrap();

    assert_eq!(lines, vec!["Sample ID,Depth,Count,Notes,Date", "S1,1,0,,"]);
    let expected = "Emptied 1 Excel error value(s), uploading them as blank cells:
  row 2, column F: #VALUE!";
    assert_eq!(error_cells.as_deref(), Some(expected));
}

#[test]
fn data_in_a_column_without_a_header_stops_the_import_and_names_the_column() {
    let err = run_pipeline(vec![
        vec![Text("S1"), Empty, Empty, Empty, Empty],
        vec![Text("S2"), Empty, Empty, Empty, Empty, Text("Super dirty")],
        vec![Text("S3"), Empty, Empty, Empty, Empty, Text("Too dirty")],
    ])
    .unwrap_err();

    let expected = "column F (first value at row 3: \"Super dirty\")";
    assert!(err.contains("without a header"), "{err}");
    assert!(err.contains(expected), "{err}");
}

#[test]
fn every_column_without_a_header_that_has_data_is_reported() {
    let err = run_sheet(&[
        vec![Text("Sample ID"), Empty, Text("Depth")],
        vec![Text("S1"), Text("left"), Number(1.0), Empty, Number(7.0)],
    ])
    .unwrap_err();

    assert!(
        err.contains("column B (first value at row 2: \"left\")"),
        "{err}"
    );
    assert!(
        err.contains("column E (first value at row 2: \"7\")"),
        "{err}"
    );
}

/// Blank columns without a header (stray spaces, or formatting that stretches the sheet) carry no
/// data, so they are dropped rather than reported as extra or duplicate columns. The columns after
/// the gap must still line up with their own headers.
#[test]
fn blank_columns_without_a_header_are_ignored() {
    let lines = run_sheet(&[
        vec![
            Text("Sample ID"),
            Text("Depth"),
            Empty,
            Text("Count"),
            Text("Notes"),
            Text("Date"),
            Text("  "),
        ],
        vec![
            Text("S1"),
            Number(1.5),
            Text("   "),
            Number(3.0),
            Text("ok"),
            Text("2024-01-02"),
            Text(" "),
        ],
    ])
    .unwrap();

    let expected = vec!["Sample ID,Depth,Count,Notes,Date", "S1,1.5,3,ok,2024-01-02"];
    assert_eq!(lines, expected);
}

/// The sheet's first used cell sets where its table starts; the column and row reported must be
/// the ones Excel shows, not positions within the table.
#[test]
fn column_without_a_header_is_reported_by_its_sheet_position_when_the_table_is_not_at_a1() {
    let err = run_sheet(&[
        vec![],
        vec![Empty, Text("Sample ID")],
        vec![Empty, Text("S1"), Text("Super dirty")],
    ])
    .unwrap_err();

    assert!(
        err.contains("column C (first value at row 3: \"Super dirty\")"),
        "{err}"
    );
}

/// With an empty first row the table starts at row 2; rows must still be reported by the number
/// Excel shows.
#[test]
fn validation_errors_name_the_row_excel_shows_when_the_table_is_not_at_row_1() {
    let mut rows = vec![vec![], headers()];
    rows.push(vec![Text("S1"), Empty, Empty, Empty, Text("08/28/2024")]);
    let err = run_sheet(&rows).unwrap_err();

    let expected = "row[3]./Date";
    assert!(err.contains("'08/28/2024' is not a valid date"), "{err}");
    assert!(err.contains(expected), "{err}");
}

#[test]
fn excel_error_cells_name_the_row_excel_shows_when_the_table_is_not_at_row_1() {
    let mut rows = vec![vec![], headers()];
    rows.push(vec![Text("S1"), Error("#N/A"), Empty, Empty, Empty]);
    let (error_cells, _) = run_sheet_with_error_cells(&rows).unwrap();

    let expected = "row 3, column B \"Depth\": #N/A";
    assert!(
        error_cells
            .as_deref()
            .unwrap_or_default()
            .contains(expected),
        "{error_cells:?}"
    );
}

/// Validate `all_rows` (the first row being the headers) and return why the rows are refused.
fn refusal(all_rows: &[Vec<common::Cell>]) -> excel_core::RowsRefused {
    let mut validator =
        ExcelValidatorBuilder::new(test_workbook(all_rows), "Sheet1", sample_schema())
            .build()
            .unwrap_or_else(|e| panic!("the rows must not stop the build: {e:#}"));
    validator
        .validate_excel()
        .expect_err("the rows must be refused")
}

/// The core writes no file: the rows failing validation come back with their report, for the
/// caller to write to the error log.
#[test]
fn rows_failing_validation_are_returned_with_their_report_for_the_error_log() {
    let mut rows = vec![headers()];
    rows.push(vec![Text("S1"), Empty, Number(3.0), Empty, Empty]);
    rows.push(vec![Text("S2"), Empty, Text("three"), Empty, Empty]);

    let refused = refusal(&rows);

    assert_eq!(refused.problems, Vec::<String>::new());
    let invalid = refused
        .invalid_rows
        .as_ref()
        .expect("row 3 must fail validation");
    let expected = "Excel validation failed with 1 error(s) across 1 row(s).";
    assert_eq!(invalid.summary, expected);
    assert!(
        invalid.report.contains("Row 3: 1 error(s)\n"),
        "{}",
        invalid.report
    );
    assert!(!invalid.report.contains("Row 2:"), "{}", invalid.report);
    // The error log stamps each entry; the report has no clock of its own
    assert!(
        !invalid.report.contains("Generated at"),
        "{}",
        invalid.report
    );
    assert_eq!(refused.to_string(), expected);
}

#[test]
fn problems_without_invalid_rows_have_no_report_for_the_error_log() {
    let rows = vec![
        vec![Text("Sample ID"), Empty],
        vec![Text("S1"), Text("stray")],
    ];

    let refused = refusal(&rows);

    assert_eq!(refused.problems.len(), 1, "{:?}", refused.problems);
    assert!(refused.invalid_rows.is_none());
    assert_eq!(refused.to_string(), refused.problems.join("\n"));
}

#[test]
fn problems_and_invalid_rows_are_reported_together_problems_first() {
    let rows = vec![
        vec![Text("Sample ID"), Text("Count"), Empty],
        vec![Text("S1"), Text("three"), Text("stray")],
    ];

    let refused = refusal(&rows);

    let invalid = refused
        .invalid_rows
        .as_ref()
        .expect("row 2 must fail validation");
    let expected = format!("{}\n{}", refused.problems.join("\n"), invalid.summary);
    assert_eq!(refused.to_string(), expected);
}

#[test]
fn a_sheet_without_rows_is_refused_as_empty() {
    let refused = refusal(&[headers()]);

    assert_eq!(
        refused.problems,
        vec!["The Excel file is empty".to_string()]
    );
    assert!(refused.invalid_rows.is_none());
}

/// The workbook is read from its bytes, so the caller reads the file and names it in its errors
#[test]
fn bytes_that_are_not_an_xlsx_workbook_are_refused() {
    let not_a_workbook = b"Sample ID,Depth\nS1,1.5\n".to_vec();

    let err = ExcelValidatorBuilder::new(not_a_workbook, "Sheet1", sample_schema())
        .build()
        .err()
        .expect("CSV bytes must be refused");

    let message = format!("{err:#}");
    assert!(
        message.starts_with("The Excel file is not a valid .xlsx workbook"),
        "{message}"
    );
}

#[test]
fn a_missing_sheet_is_refused_by_name() {
    let workbook = test_workbook(&[headers()]);

    let err = ExcelValidatorBuilder::new(workbook, "Samples", sample_schema())
        .build()
        .err()
        .expect("a missing sheet must be refused");

    let message = format!("{err:#}");
    assert!(
        message.starts_with("The sheet \"Samples\" does not exist in the Excel file"),
        "{message}"
    );
}
