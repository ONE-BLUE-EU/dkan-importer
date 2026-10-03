//! End-to-end tests through the real file pipeline: write an .xlsx, build the validator from it,
//! validate, and export the CSV that would be uploaded to DKAN.

mod common;

use common::Cell::{Empty, Error, Number, Text};
use common::{create_simple_mapping, write_test_workbook};
use importer_lib::ExcelValidatorBuilder;
use importer_lib::serde_json::json;

fn sample_schema() -> importer_lib::serde_json::Value {
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
fn run_pipeline(name: &str, rows: Vec<Vec<common::Cell>>) -> Result<Vec<String>, String> {
    let mut all_rows = vec![headers()];
    all_rows.extend(rows);
    let xlsx = write_test_workbook(name, &all_rows);
    let csv = xlsx.with_extension("csv");

    let result = (|| {
        let mut validator =
            ExcelValidatorBuilder::new(xlsx.to_str().unwrap(), "Sheet1", sample_schema())
                .build()
                .map_err(|e| format!("{e:#}"))?;
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
                create_simple_mapping(vec!["Sample ID", "Depth", "Count", "Notes", "Date"]),
            )
            .map_err(|e| e.to_string())?;
        Ok(std::fs::read_to_string(&csv)
            .unwrap()
            .lines()
            .map(String::from)
            .collect())
    })();

    let _ = std::fs::remove_file(&xlsx);
    let _ = std::fs::remove_file(&csv);
    result
}

/// DKAN infers column types from the CSV, so empty numeric cells are deliberately written as
/// numeric placeholders. This test pins that decision; the row with values shows the same columns
/// carry real data when it is present.
#[test]
fn empty_numeric_cells_are_exported_as_dkan_type_placeholders() {
    let lines = run_pipeline(
        "placeholders",
        vec![
            vec![Text("S1"), Empty, Empty, Empty, Empty],
            vec![
                Text("S2"),
                Number(1.5),
                Number(3.0),
                Text("ok"),
                Text("2024-01-02"),
            ],
        ],
    )
    .unwrap();

    assert_eq!(lines[0], "Sample ID,Depth,Count,Notes,Date");
    assert_eq!(lines[1], "S1,000000000000.000000,0,,");
    assert_eq!(lines[2], "S2,1.5,3,ok,2024-01-02");
}

#[test]
fn slash_dates_are_read_day_first() {
    let lines = run_pipeline(
        "day-first",
        vec![vec![Text("S1"), Empty, Empty, Empty, Text("03/04/2024")]],
    )
    .unwrap();

    assert_eq!(lines[1], "S1,000000000000.000000,0,,2024-04-03");
}

#[test]
fn month_first_date_is_a_validation_error() {
    let err = run_pipeline(
        "month-first",
        vec![vec![Text("S1"), Empty, Empty, Empty, Text("08/28/2024")]],
    )
    .unwrap_err();

    assert!(err.contains("'08/28/2024' is not a valid date"), "{err}");
}

#[test]
fn text_that_is_not_a_date_in_a_date_column_is_a_validation_error() {
    let err = run_pipeline(
        "not-a-date",
        vec![vec![Text("S1"), Empty, Empty, Empty, Text("n/a")]],
    )
    .unwrap_err();

    assert!(err.contains("'n/a' is not a valid date"), "{err}");
}

#[test]
fn excel_error_cell_stops_the_import_and_names_the_cell() {
    let err = run_pipeline(
        "error-cell",
        vec![
            vec![Text("S1"), Number(1.0), Empty, Empty, Empty],
            vec![Text("S2"), Error("#N/A"), Empty, Empty, Empty],
        ],
    )
    .unwrap_err();

    assert!(err.contains("#N/A"), "{err}");
    assert!(err.contains("row 3"), "{err}");
    assert!(err.contains("Depth"), "{err}");
}
