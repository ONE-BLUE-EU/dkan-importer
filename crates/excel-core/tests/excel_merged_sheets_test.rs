//! Several unpivoted sheets of one workbook merged into one CSV file: each sheet is read and
//! validated on its own, then their rows are written one sheet after the other.

mod common;

use common::Cell::{self, Empty, Number, Text};
use common::{create_simple_mapping, test_workbook_with_sheets};
use excel_core::serde_json::{Value, json};
use excel_core::{
    DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, ExcelValidator, ExcelValidatorBuilder, Unpivot,
};

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "Substance": { "type": "string" },
            "Identification Level": { "type": ["string", "null"] },
            "Detection characterisation": { "type": ["string", "null"] },
            "Concentration units": { "type": ["string", "null"] },
            "Limit of detection": { "type": ["number", "null"] },
            "Limit of quantification": { "type": ["number", "null"] },
            "Sample Tag": { "type": ["string", "null"] },
            "Concentration": { "type": ["number", "null"] }
        },
        "required": ["Substance"],
        "additionalProperties": false
    })
}

fn unpivot() -> Unpivot {
    Unpivot {
        headers_column: "Sample Tag".to_string(),
        values_column: "Concentration".to_string(),
        values_markers: DEFAULT_VALUES_MARKERS.map(String::from).to_vec(),
        marker_limits: DEFAULT_MARKER_LIMITS
            .iter()
            .map(|limit| limit.parse().unwrap())
            .collect(),
    }
}

/// The leading columns of an analytical results sheet, then `samples`
fn headers(samples: &[&'static str]) -> Vec<Cell> {
    let mut headers = vec![
        Text("Substance"),
        Text("Identification Level"),
        Text("Detection characterisation"),
        Text("Concentration units"),
        Text("Limit of detection"),
        Text("Limit of quantification"),
    ];
    headers.extend(samples.iter().map(|sample| Text(sample)));
    headers
}

/// A row of `substance` in `units`, quantified in each sample given a number
fn row(substance: &'static str, units: &'static str, samples: Vec<Cell>) -> Vec<Cell> {
    let mut row = vec![
        Text(substance),
        Number(1.0),
        Text("detected quantitatively"),
        Text(units),
        Number(0.01),
        Number(0.02),
    ];
    row.extend(samples);
    row
}

/// The workbook: a water sheet with two samples, an air sheet with one, and an air sheet whose
/// leading columns are in another order
fn workbook() -> Vec<u8> {
    let water = [
        headers(&["ST01-W", "ST02-W"]),
        row("Okadaic acid", "ng/L", vec![Number(0.5), Empty]),
        row("Domoic acid", "ng/L", vec![Number(0.7), Number(0.9)]),
    ];
    let air = [
        headers(&["F1"]),
        row("Abietic acid", "ng/L", vec![Number(3.0)]),
    ];
    let mut reordered = [
        headers(&["F1"]),
        row("Abietic acid", "ng/L", vec![Number(3.0)]),
    ];
    for row in &mut reordered {
        row.swap(0, 1);
    }
    test_workbook_with_sheets(&[("Water", &water), ("Air", &air), ("Reordered", &reordered)])
}

fn validator(workbook: &[u8], sheet: &str) -> ExcelValidator {
    let mut validator = ExcelValidatorBuilder::new(workbook.to_vec(), sheet, schema())
        .unpivot(unpivot())
        .build()
        .unwrap();
    validator.validate_excel().unwrap();
    validator
}

fn mapping() -> std::collections::HashMap<String, String> {
    create_simple_mapping(vec![
        "Substance",
        "Identification Level",
        "Sample Tag",
        "Concentration",
    ])
}

#[test]
fn the_merged_csv_has_the_headers_once_then_each_sheets_rows_in_the_order_given() {
    let workbook = workbook();
    let sheets = [validator(&workbook, "Water"), validator(&workbook, "Air")];

    let csv = ExcelValidator::merged_csv(&sheets, &mapping()).unwrap();

    let expected = "Substance,Identification Level,Detection characterisation,Concentration units,Limit of detection,Limit of quantification,Sample Tag,Concentration
Okadaic acid,1,detected quantitatively,ng/L,0.01,0.02,ST01-W,0.5
Domoic acid,1,detected quantitatively,ng/L,0.01,0.02,ST01-W,0.7
Domoic acid,1,detected quantitatively,ng/L,0.01,0.02,ST02-W,0.9
Abietic acid,1,detected quantitatively,ng/L,0.01,0.02,F1,3
";
    assert_eq!(String::from_utf8(csv).unwrap(), expected);
}

/// The single-sheet CSV is the merge of that sheet alone
#[test]
fn one_sheet_merged_is_its_own_csv() {
    let workbook = workbook();
    let water = validator(&workbook, "Water");

    let merged = ExcelValidator::merged_csv(std::slice::from_ref(&water), &mapping()).unwrap();

    assert_eq!(merged, water.to_csv(&mapping()).unwrap());
}

#[test]
fn sheets_whose_columns_differ_are_not_merged() {
    let workbook = workbook();
    let sheets = [
        validator(&workbook, "Water"),
        validator(&workbook, "Reordered"),
    ];

    let error = ExcelValidator::merged_csv(&sheets, &mapping()).unwrap_err();

    let expected = "The sheet \"Reordered\" has the columns Identification Level, Substance, Detection characterisation, Concentration units, Limit of detection, Limit of quantification, Sample Tag, Concentration, but the sheet \"Water\" has Substance, Identification Level, Detection characterisation, Concentration units, Limit of detection, Limit of quantification, Sample Tag, Concentration: only sheets with the same columns, in the same order, can be merged.";
    assert_eq!(error.to_string(), expected);
}

#[test]
fn no_sheets_are_not_merged() {
    let error = ExcelValidator::merged_csv(&[], &mapping()).unwrap_err();

    assert_eq!(error.to_string(), "No sheets to merge.");
}
