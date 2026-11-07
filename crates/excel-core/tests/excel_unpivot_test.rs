//! Unpivoting a wide sheet (one column per sample) into one row per sample cell, through the real
//! file pipeline: build an .xlsx in memory, build the validator from it, validate, and export the CSV.

mod common;

use common::Cell::{self, Empty, Error, FormulaText, Number, Text};
use common::{create_simple_mapping, test_workbook};
use excel_core::serde_json::json;
use excel_core::{
    DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, ExcelValidatorBuilder,
    IDENTIFICATION_LEVEL_FIELD, MarkerLimit, Unpivot,
};

/// Dictionary fields: two descriptors, the two unpivot targets, two limits, and one more that is in neither the
/// sheet nor the unpivot.
fn schema() -> excel_core::serde_json::Value {
    schema_with_concentration(json!({ "type": ["number", "null"] }))
}

/// The same dictionary, with "Concentration" taking text, so it can hold markers such as `<LOD`.
fn text_schema() -> excel_core::serde_json::Value {
    schema_with_concentration(json!({ "type": ["string", "null"] }))
}

fn schema_with_concentration(
    concentration: excel_core::serde_json::Value,
) -> excel_core::serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "Substance": { "type": "string" },
            "Concentration units": { "type": ["string", "null"] },
            "Detection characterisation": { "type": ["string", "null"] },
            "Identification Level": { "type": ["string", "null"] },
            "Sample Tag": { "type": ["string", "null"] },
            "Concentration": concentration,
            "Limit of detection": { "type": ["number", "null"] },
            "Limit of quantification": { "type": ["number", "null"] },
            "Classification": { "type": ["string", "null"] }
        },
        "required": ["Substance"],
        "additionalProperties": false
    })
}

fn spec() -> Unpivot {
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

#[derive(Debug)]
struct Unpivoted {
    summary: String,
    /// What the validator reports about the Excel error cells it emptied
    error_cells: Option<String>,
    /// Every change the validator made to a cell, with its reason, as the run log shows it
    changes: Vec<String>,
    /// Each open question about the data, as the run log shows it: "N cell(s): question"
    questions: Vec<String>,
    /// The exported CSV
    csv: String,
    /// [`Unpivoted::columns`] of Substance, Sample Tag and Concentration, the columns most tests
    /// are about
    csv_lines: Vec<String>,
}

impl Unpivoted {
    /// The exported CSV's `names` columns, header first, each line joined with ","
    fn columns(&self, names: &[&str]) -> Vec<String> {
        let mut reader = csv::Reader::from_reader(self.csv.as_bytes());
        let headers = reader.headers().unwrap().clone();
        let indices: Vec<usize> = names
            .iter()
            .map(|name| {
                headers
                    .iter()
                    .position(|header| header == *name)
                    .unwrap_or_else(|| panic!("no column {name} in {headers:?}"))
            })
            .collect();
        let mut lines = vec![names.join(",")];
        lines.extend(reader.records().map(|record| {
            let record = record.unwrap();
            indices
                .iter()
                .map(|&i| record.get(i).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(",")
        }));
        lines
    }
}

/// Build with `unpivot`, validate and export `rows` (the first row being the headers).
fn run(rows: &[Vec<Cell>], unpivot: Unpivot) -> Result<Unpivoted, String> {
    run_with_schema(rows, unpivot, schema())
}

/// [`run`] against `schema` instead of the default dictionary.
fn run_with_schema(
    rows: &[Vec<Cell>],
    unpivot: Unpivot,
    schema: excel_core::serde_json::Value,
) -> Result<Unpivoted, String> {
    let mut validator = ExcelValidatorBuilder::new(test_workbook(rows), "Sheet1", schema)
        .unpivot(unpivot)
        .build()
        .map_err(|e| format!("{e:#}"))?;
    let summary = validator
        .unpivot_summary()
        .ok_or("an unpivoted sheet must have a summary")?
        .to_string();
    let error_cells = validator.error_cells_summary().map(String::from);
    let changes = validator.change_lines();
    let questions = validator
        .questions()
        .iter()
        .map(|(question, cells)| format!("{cells} cell(s): {question}"))
        .collect();
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
            "Substance",
            "Identification Level",
            "Concentration units",
            "Limit of detection",
            "Limit of quantification",
            "Detection characterisation",
            "Sample Tag",
            "Concentration",
        ]))
        .map_err(|e| e.to_string())?;
    let mut unpivoted = Unpivoted {
        summary,
        error_cells,
        changes,
        questions,
        csv: String::from_utf8(csv).map_err(|e| e.to_string())?,
        csv_lines: Vec::new(),
    };
    unpivoted.csv_lines = unpivoted.columns(&["Substance", "Sample Tag", "Concentration"]);
    Ok(unpivoted)
}

fn wide_sheet() -> Vec<Vec<Cell>> {
    vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Text("ST02"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(0.88),
            Number(0.4),
        ],
        vec![],
        vec![
            Text("Azaspiracid-2"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(1.25),
            Empty,
        ],
    ]
}

#[test]
fn each_sample_cell_becomes_a_row_with_the_column_header_and_the_cell_value() {
    let unpivoted = run(&wide_sheet(), spec()).unwrap();

    // The blank source row and the blank sample cell produce no rows: a blank cell is no result
    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST01,0.88",
        "Okadaic acid,ST02,0.4",
        "Azaspiracid-2,ST01,1.25",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

#[test]
fn the_summary_names_the_kept_and_the_unpivoted_columns_and_the_row_count() {
    let unpivoted = run(&wide_sheet(), spec()).unwrap();

    let expected = "Keeping 5 column(s) A-E (Substance … Limit of quantification); unpivoting 2 column(s) F-G (ST01 … ST02) into Sample Tag / Concentration: 3 row(s); 1 blank sample cell(s) skipped.";
    assert_eq!(unpivoted.summary, expected);
}

#[test]
fn blank_sample_cells_in_every_form_produce_no_row() {
    // Empty, only spaces, and a formula giving "" (as `IF(A1="","",A1*1000)` does)
    let mut rows = marker_sheet([Empty, Text("  "), FormulaText("")]);
    rows.push(vec![
        Text("Azaspiracid-2"),
        Number(2.0),
        Text("ng/L"),
        Number(0.01),
        Number(0.02),
        Number(1.25),
        Text("  "),
        FormulaText(""),
    ]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Azaspiracid-2,ST01,1.25",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
    let expected = "Keeping 5 column(s) A-E (Substance … Limit of quantification); unpivoting 3 column(s) F-H (ST01 … ST03) into Sample Tag / Concentration: 1 row(s); 5 blank sample cell(s) skipped. Concentration takes numbers or <LOD, <LOQ, X.";
    assert_eq!(unpivoted.summary, expected);
}

#[test]
fn without_blank_sample_cells_the_summary_does_not_mention_them() {
    let rows = marker_sheet([Number(0.88), Number(0.4), Number(1.25)]);

    let unpivoted = run(&rows, spec()).unwrap();

    let expected = "Keeping 5 column(s) A-E (Substance … Limit of quantification); unpivoting 3 column(s) F-H (ST01 … ST03) into Sample Tag / Concentration: 3 row(s).";
    assert_eq!(unpivoted.summary, expected);
}

#[test]
fn text_in_the_values_column_is_a_validation_error_at_its_source_row() {
    let rows = vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Text("ST02"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(0.88),
            Number(0.4),
        ],
        vec![
            Text("Azaspiracid-2"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(1.25),
            Text("<LOD"),
        ],
    ];

    let err = run(&rows, spec()).unwrap_err();

    let expected = "row[3]./Concentration";
    assert!(err.contains(expected), "{err}");
    assert!(err.contains("<LOD"), "{err}");
    assert!(!err.contains("row[2]"), "{err}");
}

#[test]
fn a_headers_column_that_is_not_a_dictionary_field_is_refused_with_the_candidates() {
    let unpivot = Unpivot {
        headers_column: "Sample".to_string(),
        ..spec()
    };

    let err = run(&wide_sheet(), unpivot).expect_err("an unknown field must be refused");

    assert!(err.contains("headers column \"Sample\""), "{err}");
    assert!(
        err.contains(
            "Candidates: Classification, Concentration, Detection characterisation, Sample Tag"
        ),
        "{err}"
    );
}

#[test]
fn a_values_column_that_is_already_a_sheet_column_is_refused_with_the_candidates() {
    let unpivot = Unpivot {
        values_column: "Concentration units".to_string(),
        ..spec()
    };

    let err = run(&wide_sheet(), unpivot).expect_err("a sheet column must be refused");

    assert!(
        err.contains("values column \"Concentration units\""),
        "{err}"
    );
    assert!(
        err.contains(
            "Candidates: Classification, Concentration, Detection characterisation, Sample Tag"
        ),
        "{err}"
    );
}

#[test]
fn the_same_field_for_headers_and_values_is_refused() {
    let unpivot = Unpivot {
        values_column: "Sample Tag".to_string(),
        ..spec()
    };

    let err = run(&wide_sheet(), unpivot).expect_err("the same field twice must be refused");

    assert!(err.contains("must be different fields"), "{err}");
}

#[test]
fn a_sheet_that_starts_with_a_sample_column_is_refused() {
    let rows = vec![
        vec![Text("ST01"), Text("Substance")],
        vec![Number(0.88), Text("Okadaic acid")],
    ];

    let err = run(&rows, spec()).expect_err("no kept columns must be refused");

    assert!(err.contains("starts with column A \"ST01\""), "{err}");
}

#[test]
fn a_sheet_without_sample_columns_is_refused() {
    let rows = vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
        ],
    ];

    let err = run(&rows, spec()).expect_err("no sample columns must be refused");

    assert!(err.contains("no sample columns"), "{err}");
}

/// A dictionary field after the first sample column would otherwise be unpivoted as a sample.
#[test]
fn a_dictionary_field_among_the_sample_columns_is_refused() {
    let rows = vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("ST01"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST02"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Number(0.88),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(0.4),
        ],
    ];

    let err = run(&rows, spec()).expect_err("a misplaced dictionary field must be refused");

    let expected = "column D \"Concentration units\" is a data dictionary field after the first sample column C \"ST01\"";
    assert!(err.contains(expected), "{err}");
}

#[test]
fn a_sample_column_without_a_header_is_refused() {
    let rows = vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Empty,
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(0.88),
            Number(0.4),
        ],
    ];

    let err = run(&rows, spec()).expect_err("a sample column without a header must be refused");

    assert!(
        err.contains("column G, first value at row 2: Cell value: 0.4"),
        "{err}"
    );
}

/// An error in a kept cell is one broken cell, reported once, not once per sample column.
#[test]
fn excel_error_cells_are_emptied_so_a_sample_error_gives_no_row() {
    let rows = vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Classification"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Text("ST02"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Error("#N/A"),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            Number(0.88),
            Error("#DIV/0!"),
        ],
    ];

    let unpivoted = run(&rows, spec()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST01,0.88",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
    let expected = "Emptied 2 Excel error value(s), uploading them as blank cells:
  row 2, column C \"Classification\": Cell value: #N/A (Excel error)
  row 2, column H \"ST02\": Cell value: #DIV/0! (Excel error)";
    assert_eq!(unpivoted.error_cells.as_deref(), Some(expected));
    assert!(
        unpivoted
            .summary
            .ends_with("1 row(s); 1 blank sample cell(s) skipped."),
        "{}",
        unpivoted.summary
    );
}

/// Every outcome of a sample cell going to a values field that takes text.
fn marker_sheet(cells: [Cell; 3]) -> Vec<Vec<Cell>> {
    let [st01, st02, st03] = cells;
    vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Text("ST02"),
            Text("ST03"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Text("ng/L"),
            Number(0.01),
            Number(0.02),
            st01,
            st02,
            st03,
        ],
    ]
}

#[test]
fn a_text_values_field_takes_numbers_and_markers_as_written() {
    let mut rows = marker_sheet([Number(0.880999866251062), Text("<LOQ "), Text("<LOD")]);
    rows.push(vec![
        Text("Azaspiracid-2"),
        Number(2.0),
        Text("ng/L"),
        Number(0.01),
        Number(0.02),
        Text("<LOQ"),
        Empty,
        Text("0.5"),
    ]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST01,0.880999866251062",
        "Okadaic acid,ST02,<LOQ",
        "Okadaic acid,ST03,<LOD",
        "Azaspiracid-2,ST01,<LOQ",
        "Azaspiracid-2,ST03,0.5",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

#[test]
fn other_text_in_a_text_values_field_is_a_validation_error_naming_the_markers() {
    let rows = marker_sheet([Text("<LDO"), Text("n.d."), Text("NaN")]);

    let err = run_with_schema(&rows, spec(), text_schema()).unwrap_err();

    for value in ["<LDO", "n.d.", "NaN"] {
        let expected = format!(
            "row[2]./Concentration: '{value}' is not a number or one of the markers <LOD, <LOQ"
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
}

#[test]
fn given_markers_replace_the_default_ones() {
    let unpivot = Unpivot {
        values_markers: vec!["<MDL".to_string()],
        ..spec()
    };
    let rows = marker_sheet([Text("<MDL"), Text("<LOD"), Number(0.4)]);

    let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LOD' is not a number or one of the markers <MDL";
    assert!(err.contains(expected), "{err}");
    assert!(!err.contains("'<MDL'"), "{err}");
}

#[test]
fn the_summary_of_a_text_values_field_names_the_markers() {
    let unpivoted = run_with_schema(&wide_sheet(), spec(), text_schema()).unwrap();

    let expected = "Keeping 5 column(s) A-E (Substance … Limit of quantification); unpivoting 2 column(s) F-G (ST01 … ST02) into Sample Tag / Concentration: 3 row(s); 1 blank sample cell(s) skipped. Concentration takes numbers or <LOD, <LOQ, X.";
    assert_eq!(unpivoted.summary, expected);
}

/// A row with both limits and three sample cells: `cells` for an LOD of 0.1 and an LOQ of `loq`.
fn limits_sheet(loq: Cell, cells: [Cell; 3]) -> Vec<Vec<Cell>> {
    let [st01, st02, st03] = cells;
    vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Text("ST02"),
            Text("ST03"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Text("ng/L"),
            Number(0.1),
            loq,
            st01,
            st02,
            st03,
        ],
    ]
}

#[test]
fn a_marker_followed_by_the_rows_limit_is_written_as_the_marker() {
    let rows = limits_sheet(
        Number(0.25),
        [Text("<LOQ:0.25"), Text("<LOD : 0.100"), Number(0.3)],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST01,<LOQ",
        "Okadaic acid,ST02,<LOD",
        "Okadaic acid,ST03,0.3",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
    let expected_summary = "Concentration takes numbers or <LOD, <LOQ, X. 2 value(s) written as their marker alone, the number after it being the row's limit: <LOD (Limit of detection), <LOQ (Limit of quantification).";
    assert!(
        unpivoted.summary.ends_with(expected_summary),
        "{}",
        unpivoted.summary
    );
}

#[test]
fn without_such_values_the_summary_does_not_mention_limits() {
    let rows = limits_sheet(Number(0.25), [Text("<LOQ"), Text("NS"), Number(0.3)]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    assert!(
        unpivoted
            .summary
            .ends_with("Concentration takes numbers or <LOD, <LOQ, X."),
        "{}",
        unpivoted.summary
    );
}

#[test]
fn a_marker_followed_by_another_number_is_a_validation_error_naming_the_rows_limit() {
    let rows = limits_sheet(
        Number(0.25),
        [Text("<LOQ:0.3"), Text("<LOQ:abc"), Text("<LOQ:0.25")],
    );

    let err = run_with_schema(&rows, spec(), text_schema()).unwrap_err();

    for value in ["<LOQ:0.3", "<LOQ:abc"] {
        let expected = format!(
            "row[2]./Concentration: '{value}' does not end with the row's Limit of quantification (0.25)"
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
    assert!(!err.contains("row[2]./Concentration: '<LOQ:0.25'"), "{err}");
}

#[test]
fn a_marker_followed_by_a_number_in_a_row_without_its_limit_is_a_validation_error() {
    let rows = limits_sheet(Empty, [Text("<LOQ:0.25"), Text("<LOQ"), Number(0.3)]);

    let err = run_with_schema(&rows, spec(), text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LOQ:0.25' has a number, but the row has no Limit of quantification";
    assert!(err.contains(expected), "{err}");
}

/// Only markers given a limit take a number after them.
#[test]
fn a_marker_without_a_limit_followed_by_a_number_is_a_validation_error() {
    let rows = limits_sheet(Number(0.25), [Text("NS:0.25"), Text("NS"), Number(0.3)]);

    let err = run_with_schema(&rows, spec(), text_schema()).unwrap_err();

    let expected =
        "row[2]./Concentration: 'NS:0.25' is not a number or one of the markers <LOD, <LOQ";
    assert!(err.contains(expected), "{err}");
}

#[test]
fn a_limit_that_is_not_a_dictionary_field_is_refused() {
    let unpivot = Unpivot {
        marker_limits: vec!["<LOQ=Limit of quant".parse().unwrap()],
        ..spec()
    };
    let rows = limits_sheet(Number(0.25), [Text("<LOQ"), Text("NS"), Number(0.3)]);

    let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

    assert!(err.contains("\"Limit of quant\""), "{err}");
    assert!(err.contains("not a data dictionary field"), "{err}");
}

/// The default limits do not stand in the way of other markers, and do not apply without theirs.
#[test]
fn a_limit_for_a_marker_that_is_not_a_values_marker_does_not_apply() {
    let unpivot = Unpivot {
        values_markers: vec!["<LOD".to_string()],
        ..spec()
    };
    let rows = limits_sheet(Number(0.25), [Text("<LOD"), Text("<LOQ:0.25"), Number(0.3)]);

    let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LOQ:0.25' is not a number or one of the markers <LOD";
    assert!(err.contains(expected), "{err}");
}

/// A number values field takes no markers, so the limits are not checked either.
#[test]
fn a_number_values_field_ignores_the_limits() {
    let unpivot = Unpivot {
        marker_limits: vec!["<LOQ=Limit of quant".parse().unwrap()],
        ..spec()
    };
    let rows = limits_sheet(Number(0.25), [Number(0.1), Number(0.2), Number(0.3)]);

    let unpivoted = run_with_schema(
        &rows,
        unpivot,
        schema_with_concentration(json!({ "type": ["number", "null"] })),
    )
    .unwrap();

    let expected = "Okadaic acid,ST03,0.3";
    assert!(
        unpivoted.csv_lines.iter().any(|l| l == expected),
        "{:?}",
        unpivoted.csv_lines
    );
}

#[test]
fn a_marker_limit_is_read_from_marker_equals_field() {
    let limit: MarkerLimit = " <LOQ = Limit of quantification ".parse().unwrap();

    assert_eq!(limit.marker, "<LOQ");
    assert_eq!(limit.limit_field, "Limit of quantification");
}

#[test]
fn a_marker_limit_without_both_sides_is_refused() {
    for text in ["<LOQ", "<LOQ=", "=Limit of quantification", " = "] {
        let result = text.parse::<MarkerLimit>();
        assert!(result.is_err(), "{text}");
    }
}

/// ND (not detected) is written NS everywhere, so that one meaning has one marker.
#[test]
fn nd_among_the_given_markers_is_refused_as_it_gives_no_row() {
    for nd in ["ND", " nd "] {
        let unpivot = Unpivot {
            values_markers: vec!["<LOD".to_string(), nd.to_string()],
            ..spec()
        };
        let rows = marker_sheet([Text("<LOD"), Text("<LOD"), Number(0.4)]);

        let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

        let expected = format!(
            "\"{}\" cannot be a values marker: ND cells are treated as NS (no sample) and give no row.",
            nd.trim()
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
}

/// ND is treated as NS: no sample, so no row, and the run log says so for each cell.
#[test]
fn nd_cells_are_treated_as_ns_and_give_no_row() {
    let mut rows = marker_sheet([Text("ND"), Text(" nd "), Text("NS")]);
    rows.push(vec![
        Text("Azaspiracid-2"),
        Number(2.0),
        Text("ng/L"),
        Number(0.01),
        Number(0.02),
        Number(1.25),
        Empty,
        Empty,
    ]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Azaspiracid-2,ST01,1.25",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
    let expected = vec![
        "row 2, column F \"ST01\": Cell value: \"ND\". Not imported, because it is taken as NS: no sample was taken.",
        "row 2, column G \"ST02\": Cell value: \" nd \". Not imported, because it is taken as NS: no sample was taken.",
        "row 2, column H \"ST03\": Cell value: \"NS\". Not imported, because no sample was taken.",
    ];
    assert_eq!(unpivoted.changes, expected);
    assert!(
        unpivoted.summary.contains(
            ": 1 row(s); 2 blank sample cell(s) skipped; 3 NS or ND (no sample) cell(s) skipped."
        ),
        "{}",
        unpivoted.summary
    );
}

/// <LQ is written <LOQ everywhere.
#[test]
fn lq_among_the_given_markers_is_refused_pointing_to_loq() {
    let unpivot = Unpivot {
        values_markers: vec!["<LOQ".to_string(), "<LQ".to_string()],
        ..spec()
    };
    let rows = marker_sheet([Text("<LOQ"), Text("<LOQ"), Number(0.4)]);

    let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

    let expected = "\"<LQ\" cannot be a values marker: <LQ maps to <LOQ. Use <LOQ, and change <LQ to <LOQ in the source file, for consistency.";
    assert!(err.contains(expected), "{err}");
}

/// Also when followed by a limit, as `<LOQ` may be.
#[test]
fn lq_in_a_sample_cell_is_a_validation_error_pointing_to_loq() {
    let rows = limits_sheet(
        Number(0.25),
        [Text("<LQ"), Text("<lq : 0.25"), Text("<LOQ:0.25")],
    );

    let err = run_with_schema(&rows, spec(), text_schema()).unwrap_err();

    for value in ["<LQ", "<lq : 0.25"] {
        let expected = format!(
            "row[2]./Concentration: '{value}' is not allowed: <LQ maps to <LOQ. Change it to <LOQ in the source file, for consistency."
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
    assert!(!err.contains("'<LOQ:0.25'"), "{err}");
}

/// NQ (not quantified) is written <LOQ everywhere, like <LQ.
#[test]
fn nq_among_the_given_markers_is_refused_pointing_to_loq() {
    let unpivot = Unpivot {
        values_markers: vec!["<LOQ".to_string(), "NQ".to_string()],
        ..spec()
    };
    let rows = marker_sheet([Text("<LOQ"), Text("<LOQ"), Number(0.4)]);

    let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

    let expected = "\"NQ\" cannot be a values marker: NQ maps to <LOQ. Use <LOQ, and change NQ to <LOQ in the source file, for consistency.";
    assert!(err.contains(expected), "{err}");
}

/// Also when followed by a limit, as `<LOQ` may be.
#[test]
fn nq_in_a_sample_cell_is_a_validation_error_pointing_to_loq() {
    let rows = limits_sheet(
        Number(0.25),
        [Text("NQ"), Text("nq : 0.25"), Text("<LOQ:0.25")],
    );

    let err = run_with_schema(&rows, spec(), text_schema()).unwrap_err();

    for value in ["NQ", "nq : 0.25"] {
        let expected = format!(
            "row[2]./Concentration: '{value}' is not allowed: NQ maps to <LOQ. Change it to <LOQ in the source file, for consistency."
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
    assert!(!err.contains("'<LOQ:0.25'"), "{err}");
}

/// Markers match ignoring case, and are written uppercase, so one marker reads one way.
#[test]
fn markers_match_ignoring_case_and_are_written_uppercase() {
    let unpivot = Unpivot {
        values_markers: vec!["<LOD".to_string(), "<LOQ".to_string()],
        ..spec()
    };
    let mut rows = marker_sheet([Text("<loq"), Text(" <LoQ "), Text("<lod")]);
    rows.push(vec![
        Text("Azaspiracid-2"),
        Number(2.0),
        Text("ng/L"),
        Number(0.01),
        Number(0.02),
        Text("<LOQ"),
        Text(" <loq"),
        Text("<LoD"),
    ]);

    let unpivoted = run_with_schema(&rows, unpivot, text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST01,<LOQ",
        "Okadaic acid,ST02,<LOQ",
        "Okadaic acid,ST03,<LOD",
        "Azaspiracid-2,ST01,<LOQ",
        "Azaspiracid-2,ST02,<LOQ",
        "Azaspiracid-2,ST03,<LOD",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

/// A marker given in lowercase is the uppercase marker: listed and written uppercase.
#[test]
fn a_marker_given_in_lowercase_is_taken_uppercase() {
    let unpivot = Unpivot {
        values_markers: vec!["<mdl".to_string()],
        ..spec()
    };
    let rows = marker_sheet([Text("<mdl"), Text("<MDL"), Text("<LDO")]);

    let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LDO' is not a number or one of the markers <MDL";
    assert!(err.contains(expected), "{err}");
    assert!(!err.contains("'<mdl'"), "{err}");
    assert!(!err.contains("'<MDL'"), "{err}");
}

/// Also when followed by the row's limit, and with a limit marker given in lowercase.
#[test]
fn a_marker_with_the_rows_limit_matches_ignoring_case() {
    let unpivot = Unpivot {
        values_markers: vec!["<loq".to_string()],
        marker_limits: vec!["<loq=Limit of quantification".parse().unwrap()],
        ..spec()
    };
    let rows = limits_sheet(
        Number(0.25),
        [Text("<loq:0.25"), Text("<Loq : 0.25"), Text("<LOQ")],
    );

    let unpivoted = run_with_schema(&rows, unpivot, text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST01,<LOQ",
        "Okadaic acid,ST02,<LOQ",
        "Okadaic acid,ST03,<LOQ",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

#[test]
fn a_marker_limit_given_in_lowercase_parses_uppercase() {
    let limit: MarkerLimit = "<loq=Limit of quantification".parse().unwrap();

    let expected = MarkerLimit {
        marker: "<LOQ".to_string(),
        limit_field: "Limit of quantification".to_string(),
    };
    assert_eq!(limit, expected);
}

/// A row of [`levels_sheet`] with `level` and the result `st01`
fn level_row(level: Cell, st01: Cell) -> Vec<Cell> {
    vec![
        Text("Okadaic acid"),
        level,
        Text("ng/L"),
        Number(0.01),
        Number(0.02),
        st01,
    ]
}

/// A sheet whose rows have `levels` as their identification level, each with one result.
fn levels_sheet(levels: Vec<Cell>) -> Vec<Vec<Cell>> {
    let mut rows = vec![vec![
        Text("Substance"),
        Text(IDENTIFICATION_LEVEL_FIELD),
        Text("Concentration units"),
        Text("Limit of detection"),
        Text("Limit of quantification"),
        Text("ST01"),
    ]];
    rows.extend(
        levels
            .into_iter()
            .map(|level| level_row(level, Number(0.88))),
    );
    rows
}

#[test]
fn a_dictionary_without_the_identification_level_field_is_refused() {
    let mut schema = schema();
    schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove(IDENTIFICATION_LEVEL_FIELD)
        .expect("the test dictionary has the field to remove");
    let rows = vec![
        vec![Text("Substance"), Text("ST01")],
        vec![Text("Okadaic acid"), Number(0.88)],
    ];

    let err = run_with_schema(&rows, spec(), schema).unwrap_err();

    let expected = "the data dictionary has no field \"Identification Level\"";
    assert!(err.contains(expected), "{err}");
}

#[test]
fn a_sheet_without_the_identification_level_column_is_refused() {
    let rows = vec![
        vec![Text("Substance"), Text("ST01")],
        vec![Text("Okadaic acid"), Number(0.88)],
    ];

    let err = run(&rows, spec()).unwrap_err();

    let expected = "it has no \"Identification Level\" column before the sample columns";
    assert!(err.contains(expected), "{err}");
}

const LEVEL_COLUMNS: [&str; 4] = [
    "Substance",
    "Identification Level",
    "Sample Tag",
    "Concentration",
];

/// Any number, as the sheet's own level
#[test]
fn an_identification_level_that_is_a_number_is_kept_as_written() {
    let rows = levels_sheet(vec![Number(2.0), Number(1.0), Number(2.5)]);

    let unpivoted = run(&rows, spec()).unwrap();

    let expected = vec![
        "Substance,Identification Level,Sample Tag,Concentration",
        "Okadaic acid,2,ST01,0.88",
        "Okadaic acid,1,ST01,0.88",
        "Okadaic acid,2.5,ST01,0.88",
    ];
    assert_eq!(unpivoted.columns(&LEVEL_COLUMNS), expected);
    assert_eq!(unpivoted.changes, Vec::<String>::new());
}

/// A blank level is taken as 2, and listed as a change; only in rows with results, the others
/// giving no row
#[test]
fn a_blank_identification_level_is_filled_with_2_and_listed() {
    let mut rows = levels_sheet(vec![Empty, Text("  "), Number(1.0)]);
    rows.push(level_row(Empty, Empty));

    let unpivoted = run(&rows, spec()).unwrap();

    let expected = vec![
        "Substance,Identification Level,Sample Tag,Concentration",
        "Okadaic acid,2,ST01,0.88",
        "Okadaic acid,2,ST01,0.88",
        "Okadaic acid,1,ST01,0.88",
    ];
    assert_eq!(unpivoted.columns(&LEVEL_COLUMNS), expected);
    let expected = vec![
        "row 2, column B \"Identification Level\": Cell value: blank. Imported as \"2\", because a blank Identification Level is taken as 2.",
        "row 3, column B \"Identification Level\": Cell value: \"  \". Imported as \"2\", because a blank Identification Level is taken as 2.",
    ];
    assert_eq!(unpivoted.changes, expected);
}

/// Text is refused, even text reading 2, which Excel shows the same as the number
#[test]
fn rows_whose_identification_level_is_not_a_number_stop_the_run_listing_each() {
    let mut rows = levels_sheet(vec![Text("2"), Text("2A"), Number(2.0), Text("n/a")]);
    // A row without results gives no rows, so its identification level does not matter
    rows.push(level_row(Text("2A"), Empty));
    rows.push(level_row(Text("2A"), Text("NS")));

    let err = run(&rows, spec()).unwrap_err();

    let expected = "The sheet \"Sheet1\" has 3 row(s) whose Identification Level is not a number. \
A blank one is taken as 2: correct these cells in the source file and try again:
  row 2: Cell value: \"2\"
  row 3: Cell value: \"2A\"
  row 5: Cell value: \"n/a\"";
    assert!(err.contains(expected), "{err}");
    for row in ["row 4", "row 6", "row 7"] {
        assert!(!err.contains(row), "{err}");
    }
}

/// NS means no sample was taken, so there is no result to import.
#[test]
fn ns_cells_give_no_row_and_are_counted() {
    let rows = marker_sheet([Text("NS"), Text(" ns "), Number(0.4)]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST03,0.4",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
    let expected = "Keeping 5 column(s) A-E (Substance … Limit of quantification); unpivoting 3 column(s) F-H (ST01 … ST03) into Sample Tag / Concentration: 1 row(s); 2 NS or ND (no sample) cell(s) skipped. Concentration takes numbers or <LOD, <LOQ, X.";
    assert_eq!(unpivoted.summary, expected);
}

#[test]
fn ns_cells_give_no_row_also_with_a_number_values_field() {
    let rows = marker_sheet([Text("NS"), Number(0.88), Number(0.4)]);

    let unpivoted = run(&rows, spec()).unwrap();

    let expected = vec![
        "Substance,Sample Tag,Concentration",
        "Okadaic acid,ST02,0.88",
        "Okadaic acid,ST03,0.4",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

#[test]
fn ns_among_the_given_markers_is_refused_as_it_gives_no_row() {
    for ns in ["NS", " ns "] {
        let unpivot = Unpivot {
            values_markers: vec!["<LOD".to_string(), ns.to_string()],
            ..spec()
        };
        let rows = marker_sheet([Text("<LOD"), Number(0.88), Number(0.4)]);

        let err = run_with_schema(&rows, unpivot, text_schema()).unwrap_err();

        let expected = format!(
            "\"{}\" cannot be a values marker: NS (no sample) cells give no row.",
            ns.trim()
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
}

/// Every change to a sample cell is listed, in sheet order, with its reason; cells taken as
/// written are not, and neither are blank cells, which the summary only counts: a sheet has
/// thousands.
#[test]
fn every_change_to_a_sample_cell_is_listed_with_its_reason() {
    let mut rows = limits_sheet(Number(0.25), [Text("<loq:0.25"), Text(" ns "), Empty]);
    rows.push(vec![
        Text("Azaspiracid-2"),
        Number(2.0),
        Text("ng/L"),
        Number(0.1),
        Number(0.25),
        Error("#VALUE!"),
        Text("<lod"),
        Text("<LOD"),
    ]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "row 2, column F \"ST01\": Cell value: \"<loq:0.25\". Imported as \"<LOQ\", because the number after the marker is the row's Limit of quantification.",
        "row 2, column G \"ST02\": Cell value: \" ns \". Not imported, because no sample was taken.",
        "row 3, column F \"ST01\": Cell value: #VALUE! (Excel error). Imported as a blank cell, because an Excel error is a failed calculation.",
        "row 3, column G \"ST02\": Cell value: \"<lod\". Imported as \"<LOD\", because markers are written in capitals.",
    ];
    assert_eq!(unpivoted.changes, expected);
}

#[test]
fn a_sheet_taken_as_written_lists_no_changes() {
    let rows = limits_sheet(Number(0.25), [Text("<LOQ"), Text(" <LOD "), Number(0.3)]);

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    assert_eq!(unpivoted.changes, Vec::<String>::new());
}

// Detection characterisation: worked out for each result from the row's units and limits

/// A sheet with the source's own Detection characterisation column (C), and samples `tags` from
/// column G. Each of `rows` is: detection characterisation, units, LOD, LOQ, then its results.
fn analytical_sheet(tags: &[&'static str], rows: Vec<Vec<Cell>>) -> Vec<Vec<Cell>> {
    let mut header = vec![
        Text("Substance"),
        Text("Identification Level"),
        Text("Detection characterisation"),
        Text("Concentration units"),
        Text("Limit of detection"),
        Text("Limit of quantification"),
    ];
    header.extend(tags.iter().map(|tag| Text(tag)));
    let mut sheet = vec![header];
    sheet.extend(rows.into_iter().map(|row| {
        let mut cells = vec![Text("Okadaic acid"), Number(2.0)];
        cells.extend(row);
        cells
    }));
    sheet
}

const RESULTS: [&str; 3] = ["Sample Tag", "Concentration", "Detection characterisation"];

#[test]
fn ng_l_results_are_characterised_against_the_rows_limits() {
    let tags = [
        "ST01", "ST02", "ST03", "ST04", "ST05", "ST06", "ST07", "ST08",
    ];
    let rows = analytical_sheet(
        &tags,
        vec![
            vec![
                Empty,
                Text("ng/L"),
                Number(0.1),
                Number(0.25),
                Number(0.3),
                Number(0.25),
                Number(0.2),
                Number(0.1),
                Number(0.05),
                Text("<LOD"),
                Text("<LOQ"),
                Text("<LOQ:0.25"),
            ],
            // Units match ignoring case and spaces
            vec![
                Empty,
                Text(" NG/l "),
                Number(0.1),
                Number(0.25),
                Number(0.3),
            ],
        ],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Sample Tag,Concentration,Detection characterisation",
        "ST01,0.3,detected quantitatively",
        "ST02,0.25,detected quantitatively",
        "ST03,0.2,detected, not quantified",
        "ST04,0.1,detected, not quantified",
        "ST05,0.05,not detected",
        "ST06,<LOD,not detected",
        "ST07,<LOQ,detected, not quantified",
        "ST08,<LOQ,detected, not quantified",
        "ST01,0.3,detected quantitatively",
    ];
    assert_eq!(unpivoted.columns(&RESULTS), expected);
    let expected = vec![
        "4 cell(s): Results between the Limit of detection and the Limit of quantification, and \"<LOQ\" results, are imported as \"detected, not quantified\". The template's dropdown list does not have this value: it has \"detected semiquantitatively\". Which should be used?",
    ];
    assert_eq!(unpivoted.questions, expected);
}

#[test]
fn x_is_detected_qualitatively_in_peak_area_rows_and_in_rows_without_units() {
    let rows = analytical_sheet(
        &["ST01", "ST02"],
        vec![
            vec![Empty, Text("Peak Area"), Empty, Empty, Text("X"), Text("x")],
            vec![Empty, Text("peak area"), Empty, Empty, Text("X"), Empty],
            vec![Empty, Empty, Empty, Empty, Text("X"), Empty],
        ],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Sample Tag,Concentration,Detection characterisation",
        "ST01,X,detected qualitatively",
        "ST02,X,detected qualitatively",
        "ST01,X,detected qualitatively",
        "ST01,X,detected qualitatively",
    ];
    assert_eq!(unpivoted.columns(&RESULTS), expected);
    let expected = vec![
        "1 cell(s): X results in rows without Concentration units are taken as Peak Area (detected qualitatively), without writing the units. Is that right?",
    ];
    assert_eq!(unpivoted.questions, expected);
}

#[test]
fn items_are_whole_numbers_detected_quantitatively() {
    let rows = analytical_sheet(
        &["ST01", "ST02"],
        vec![vec![
            Empty,
            Text("items"),
            Empty,
            Empty,
            Number(3.0),
            Number(0.0),
        ]],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Sample Tag,Concentration,Detection characterisation",
        "ST01,3,detected quantitatively",
        "ST02,0,detected quantitatively",
    ];
    assert_eq!(unpivoted.columns(&RESULTS), expected);
    assert_eq!(unpivoted.questions, Vec::<String>::new());
}

#[test]
fn items_that_are_not_whole_numbers_give_no_row_and_a_question() {
    let rows = analytical_sheet(
        &["ST01", "ST02", "ST03"],
        vec![vec![
            Empty,
            Text("items"),
            Empty,
            Empty,
            Number(1.5),
            Text("<LOD"),
            Number(2.0),
        ]],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Sample Tag,Concentration,Detection characterisation",
        "ST03,2,detected quantitatively",
    ];
    assert_eq!(unpivoted.columns(&RESULTS), expected);
    let expected = vec![
        "row 2, column G \"ST01\": Cell value: 1.5. Not imported, because the row counts items, and this is not a whole number of 0 or more. See Q1.",
        "row 2, column H \"ST02\": Cell value: \"<LOD\". Not imported, because the row counts items, and this is not a whole number of 0 or more. See Q1.",
    ];
    assert_eq!(unpivoted.changes, expected);
    let expected = vec![
        "2 cell(s): Items results that are not whole numbers of 0 or more are not imported. What do they mean?",
    ];
    assert_eq!(unpivoted.questions, expected);
}

/// X is a Peak Area result, so in an ng/L row the units or the result are wrong.
#[test]
fn x_in_an_ng_l_row_gives_no_row_and_a_question() {
    let rows = analytical_sheet(
        &["ST01", "ST02"],
        vec![vec![
            Empty,
            Text("ng/L"),
            Number(0.1),
            Number(0.25),
            Text("X"),
            Number(0.3),
        ]],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Sample Tag,Concentration,Detection characterisation",
        "ST02,0.3,detected quantitatively",
    ];
    assert_eq!(unpivoted.columns(&RESULTS), expected);
    let expected = vec![
        "row 2, column G \"ST01\": Cell value: \"X\". Not imported, because \"X\" is a Peak Area result, but the row is in ng/L. See Q1.",
    ];
    assert_eq!(unpivoted.changes, expected);
    let expected = vec![
        "1 cell(s): X results in ng/L rows are not imported, as X is a Peak Area result. Are the units or the results wrong?",
    ];
    assert_eq!(unpivoted.questions, expected);
}

/// What is not clear yet is not imported, and asked about once per kind, with its cell count.
#[test]
fn unclear_results_give_no_row_and_a_question() {
    let rows = analytical_sheet(
        &["ST01"],
        vec![
            vec![Empty, Text("ng/L"), Number(0.1), Empty, Number(0.3)],
            vec![Empty, Text("Peak Area"), Empty, Empty, Number(1234.0)],
            vec![Empty, Text("ug/kg"), Number(0.1), Number(0.25), Number(0.3)],
            vec![Empty, Empty, Empty, Empty, Number(0.3)],
            vec![Empty, Text("Peak Area"), Empty, Empty, Text("<LOD")],
            vec![
                Empty,
                Text("ug/kg"),
                Number(0.1),
                Number(0.25),
                Text("<LOQ"),
            ],
            // A sheet without any result to import is refused as empty
            vec![Empty, Text("ng/L"), Number(0.1), Number(0.25), Number(0.3)],
        ],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Sample Tag,Concentration,Detection characterisation",
        "ST01,0.3,detected quantitatively",
    ];
    assert_eq!(unpivoted.columns(&RESULTS), expected);
    let expected = vec![
        "row 2, column G \"ST01\": Cell value: 0.3. Not imported, because the ng/L row has no Limit of detection or no Limit of quantification, so its results cannot be characterised. See Q1.",
        "row 3, column G \"ST01\": Cell value: 1234. Not imported, because a number in a Peak Area row has no meaning yet. See Q2.",
        "row 4, column G \"ST01\": Cell value: 0.3. Not imported, because Concentration units \"ug/kg\" are not imported. See Q3.",
        "row 5, column G \"ST01\": Cell value: 0.3. Not imported, because a number in a row without Concentration units has no meaning yet. See Q5.",
        "row 6, column G \"ST01\": Cell value: \"<LOD\". Not imported, because \"<LOD\" in a row with Concentration units \"Peak Area\" has no meaning yet. See Q4.",
        "row 7, column G \"ST01\": Cell value: \"<LOQ\". Not imported, because Concentration units \"ug/kg\" are not imported. See Q3.",
    ];
    assert_eq!(unpivoted.changes, expected);
    let expected = vec![
        "1 cell(s): ng/L rows without a Limit of detection or a Limit of quantification are not imported, as their results cannot be characterised. Should they be imported, and how?",
        "1 cell(s): Numbers in Peak Area rows are not imported. Should they be imported as the Concentration, with which detection characterisation, and should a field mark them as peak areas?",
        "2 cell(s): Rows with Concentration units \"ug/kg\" are not imported. How should they be handled?",
        "1 cell(s): \"<LOD\" in a row with Concentration units \"Peak Area\" is not imported. What does it mean?",
        "1 cell(s): A number in a row without Concentration units is not imported. What does it mean?",
    ];
    assert_eq!(unpivoted.questions, expected);
}

/// The characterisation replaces the sheet's own; where they differ, the run log says why, naming
/// what the cell holds as written.
#[test]
fn a_characterisation_that_differs_from_the_sheets_is_listed_with_what_the_cell_holds() {
    let rows = analytical_sheet(
        &["ST01", "ST02", "ST03"],
        vec![
            vec![
                Text("not detected"),
                Text("ng/L"),
                Number(0.1),
                Number(0.25),
                Number(0.3),
                Text("<loq"),
                Number(0.2),
            ],
            vec![
                Text("detected quantitatively"),
                Text("ng/L"),
                Number(0.1),
                Number(0.25),
                Number(0.05),
                Text("<LOD"),
                Empty,
            ],
            vec![
                Text("not detected"),
                Text("Peak Area"),
                Empty,
                Empty,
                Text("X"),
                Empty,
                Empty,
            ],
            vec![
                Text("not detected"),
                Empty,
                Empty,
                Empty,
                Text("X"),
                Empty,
                Empty,
            ],
            vec![
                Text("not detected"),
                Text("items"),
                Empty,
                Empty,
                Number(3.0),
                Empty,
                Empty,
            ],
        ],
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "row 2, column G \"ST01\": Cell value: 0.3. Imported with Detection characterisation \"detected quantitatively\" instead of the sheet's \"not detected\", because the value is at least the Limit of quantification (0.25).",
        "row 2, column H \"ST02\": Cell value: \"<loq\". Imported as \"<LOQ\", because markers are written in capitals.",
        "row 2, column H \"ST02\": Cell value: \"<loq\". Imported with Detection characterisation \"detected, not quantified\" instead of the sheet's \"not detected\", because \"<LOQ\" means detected, but below the Limit of quantification. See Q2.",
        "row 2, column I \"ST03\": Cell value: 0.2. Imported with Detection characterisation \"detected, not quantified\" instead of the sheet's \"not detected\", because the value is between the Limit of detection (0.1) and the Limit of quantification (0.25). See Q2.",
        "row 3, column G \"ST01\": Cell value: 0.05. Imported with Detection characterisation \"not detected\" instead of the sheet's \"detected quantitatively\", because the value is below the Limit of detection (0.1).",
        "row 3, column H \"ST02\": Cell value: \"<LOD\". Imported with Detection characterisation \"not detected\" instead of the sheet's \"detected quantitatively\", because \"<LOD\" means below the Limit of detection.",
        "row 4, column G \"ST01\": Cell value: \"X\". Imported with Detection characterisation \"detected qualitatively\" instead of the sheet's \"not detected\", because \"X\" in a Peak Area row means detected, without a concentration.",
        "row 5, column G \"ST01\": Cell value: \"X\". Imported with Detection characterisation \"detected qualitatively\" instead of the sheet's \"not detected\", because the row has no Concentration units, so \"X\" is taken as a Peak Area result. See Q1.",
        "row 6, column G \"ST01\": Cell value: 3. Imported with Detection characterisation \"detected quantitatively\" instead of the sheet's \"not detected\", because the row counts items.",
    ];
    assert_eq!(unpivoted.changes, expected);
}

#[test]
fn a_dictionary_without_the_detection_characterisation_field_is_refused() {
    let mut schema = text_schema();
    schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("Detection characterisation")
        .expect("the test dictionary has the field to remove");

    let err = run_with_schema(&wide_sheet(), spec(), schema).unwrap_err();

    let expected = "the data dictionary has no field \"Detection characterisation\", which analytical results need";
    assert!(err.contains(expected), "{err}");
}

#[test]
fn a_sheet_without_the_concentration_units_column_is_refused() {
    let rows = vec![
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
        ],
        vec![
            Text("Okadaic acid"),
            Number(2.0),
            Number(0.1),
            Number(0.25),
            Number(0.3),
        ],
    ];

    let err = run(&rows, spec()).unwrap_err();

    let expected = "it has no \"Concentration units\" column before the sample columns";
    assert!(err.contains(expected), "{err}");
}

/// Every problem that stops the run is reported together, after the changes and questions are
/// known, so one run shows the data owners everything to correct and answer.
#[test]
fn every_problem_is_reported_in_one_run_with_the_changes_and_questions() {
    let mut rows = analytical_sheet(
        &["ST01", "ST02"],
        vec![
            vec![
                Empty,
                Text("ng/L"),
                Number(0.1),
                Number(0.25),
                Text("X"),
                Number(0.2),
            ],
            vec![Empty, Text("ng/mL"), Empty, Empty, Number(0.3), Empty],
            vec![Empty, Text("items"), Empty, Empty, Number(1.5), Number(2.0)],
        ],
    );
    // Row 4's identification level is not a number
    rows[3][1] = Text("2A");
    let mut validator = ExcelValidatorBuilder::new(test_workbook(&rows), "Sheet1", text_schema())
        .unpivot(spec())
        .build()
        .unwrap_or_else(|e| panic!("the problems must not stop the build: {e:#}"));
    let changes: Vec<String> = validator.change_lines();
    let questions: Vec<String> = validator
        .questions()
        .iter()
        .map(|(question, cells)| format!("{cells} cell(s): {question}"))
        .collect();
    let err = validator
        .validate_excel()
        .expect_err("the problems must stop the run")
        .to_string();

    let expected = vec![
        "row 2, column G \"ST01\": Cell value: \"X\". Not imported, because \"X\" is a Peak Area result, but the row is in ng/L. See Q1.",
        "row 4, column G \"ST01\": Cell value: 1.5. Not imported, because the row counts items, and this is not a whole number of 0 or more. See Q2.",
    ];
    assert_eq!(changes, expected);
    // X in ng/L, the 1.5 items, and "detected, not quantified" for row 2's 0.2
    assert_eq!(questions.len(), 3, "{questions:?}");
    let expected = "The sheet \"Sheet1\" has 1 row(s) whose Identification Level is not a number. A blank one is taken as 2: correct these cells in the source file and try again:
  row 4: Cell value: \"2A\"";
    assert!(err.contains(expected), "{err}");
    assert!(
        err.contains("has 1 ng/mL row(s) whose values no ng/L row holds"),
        "{err}"
    );
}

/// What the validator built from `rows` says: its ng/mL rows as the run log lists them, its
/// changes, and the error of the run, if it stops.
struct NgPerMlRun {
    ng_per_ml_rows: Vec<String>,
    changes: Vec<String>,
    summary: String,
    error: Option<String>,
}

fn run_ng_per_ml(rows: &[Vec<Cell>]) -> NgPerMlRun {
    let mut validator = ExcelValidatorBuilder::new(test_workbook(rows), "Sheet1", text_schema())
        .unpivot(spec())
        .build()
        .unwrap_or_else(|e| panic!("ng/mL rows must not stop the build: {e:#}"));
    NgPerMlRun {
        ng_per_ml_rows: validator
            .ng_per_ml_rows()
            .iter()
            .map(ToString::to_string)
            .collect(),
        changes: validator.change_lines(),
        summary: validator.unpivot_summary().unwrap_or_default().to_string(),
        error: validator.validate_excel().err().map(|e| e.to_string()),
    }
}

/// Only ng/L is imported. Some sheets keep ng/mL rows beside the ng/L rows calculated from them
/// by formulas, so an ng/mL row is skipped when an ng/L row holds its values: the same text
/// (ignoring case and spaces), and its numbers x1000 in the limits and the sample columns. Where
/// the ng/mL row is blank, or holds an Excel error, the ng/L row may hold anything.
#[test]
fn ng_ml_rows_are_skipped_when_an_ng_l_row_holds_their_values() {
    let rows = analytical_sheet(
        &["ST01", "ST02", "ST03"],
        vec![
            // Row 2: the ng/L row calculated from row 3, with the values Excel saved, which
            // agree with x1000 to 10 significant digits
            vec![
                Empty,
                Text("ng/L"),
                Number(0.086),
                Number(0.344),
                Number(34.79658006),
                Text("<LOQ"),
                Empty,
            ],
            vec![
                Empty,
                Text("ng/mL"),
                Number(8.599999999999999e-05),
                Number(0.00034399999999999996),
                Number(0.03479658005914394),
                Text("<loq"),
                Error("#VALUE!"),
            ],
            // Row 4: in ng/mL, ignoring case and spaces; its ng/L row comes after it
            vec![
                Empty,
                Text(" NG/ml "),
                Number(0.0001),
                Number(0.00025),
                Empty,
                Number(0.0003),
                Empty,
            ],
            vec![
                Text("detected quantitatively"),
                Text("ng/L"),
                Number(0.1),
                Number(0.25),
                Empty,
                Number(0.3),
                Empty,
            ],
        ],
    );

    let run = run_ng_per_ml(&rows);

    let expected = vec![
        "row 3: skipped: in ng/mL; its values are in the ng/L row 2, which is imported",
        "row 4: skipped: in ng/mL; its values are in the ng/L row 5, which is imported",
    ];
    assert_eq!(run.ng_per_ml_rows, expected);
    assert_eq!(run.error, None);
    // Row 3's #VALUE! is not emptied for the upload: the row is not uploaded
    assert_eq!(run.changes, Vec::<String>::new());
    assert!(
        run.summary.contains("3 row(s); 2 ng/mL row(s) skipped"),
        "{}",
        run.summary
    );

    let unpivoted = run_with_schema(&rows, spec(), text_schema()).unwrap();
    let expected = vec![
        "Substance,Sample Tag,Concentration,Concentration units",
        "Okadaic acid,ST01,34.79658006,ng/L",
        "Okadaic acid,ST02,<LOQ,ng/L",
        "Okadaic acid,ST02,0.3,ng/L",
    ];
    assert_eq!(
        unpivoted.columns(&[
            "Substance",
            "Sample Tag",
            "Concentration",
            "Concentration units"
        ]),
        expected
    );
}

/// An ng/mL row whose values no ng/L row holds is still skipped, but stops the run, naming the
/// ng/L row closest to it and its first difference, as a formula in it may point at the wrong row.
#[test]
fn an_ng_ml_row_without_an_equivalent_ng_l_row_stops_the_run_naming_the_closest() {
    let rows = analytical_sheet(
        &["ST01", "ST02"],
        vec![
            vec![
                Empty,
                Text("ng/mL"),
                Number(0.0001),
                Number(0.00025),
                Number(0.0003),
                Text("<LOD"),
            ],
            // Rows 3 and 5: the limit and the result x100, and another marker
            vec![
                Empty,
                Text("ng/L"),
                Number(0.1),
                Number(0.025),
                Number(0.03),
                Text("<LOQ"),
            ],
            // Row 4: the limit of quantification is missing
            vec![
                Empty,
                Text("ng/L"),
                Number(0.1),
                Empty,
                Number(0.3),
                Text("<LOD"),
            ],
            vec![
                Empty,
                Text("ng/L"),
                Number(0.1),
                Number(0.025),
                Number(0.03),
                Text("<LOQ"),
            ],
        ],
    );

    let run = run_ng_per_ml(&rows);

    let line = "row 2: skipped: in ng/mL, but no ng/L row has its values (x1000 in the limits and the sample columns); the closest, row 4, differs in 1 cell(s), first at row 4, column F \"Limit of quantification\": Cell value: blank; expected 0.25";
    assert_eq!(run.ng_per_ml_rows, vec![line]);
    let error = run.error.expect("the missing ng/L row must stop the run");
    let expected = format!(
        "The sheet \"Sheet1\" has 1 ng/mL row(s) whose values no ng/L row holds. Only ng/L is imported, so each ng/mL row is skipped, and its values must be in an ng/L row: correct the ng/L rows in the source file and try again:\n  {line}"
    );
    assert!(error.contains(&expected), "{error}");
}

#[test]
fn an_ng_ml_row_in_a_sheet_without_ng_l_rows_stops_the_run() {
    let rows = analytical_sheet(
        &["ST01"],
        vec![
            vec![
                Empty,
                Text("ng/mL"),
                Number(0.0001),
                Number(0.00025),
                Number(0.0003),
            ],
            vec![Empty, Text("Peak Area"), Empty, Empty, Text("X")],
        ],
    );

    let run = run_ng_per_ml(&rows);

    let line = "row 2: skipped: in ng/mL, but the sheet has no ng/L row to hold its values";
    assert_eq!(run.ng_per_ml_rows, vec![line]);
    let error = run.error.expect("the missing ng/L row must stop the run");
    assert!(error.contains(&format!("\n  {line}")), "{error}");
}

/// An ng/mL row is not imported, so its identification level is not checked; its ng/L row's is.
#[test]
fn the_identification_level_of_an_ng_ml_row_is_not_checked() {
    let mut rows = analytical_sheet(
        &["ST01"],
        vec![
            vec![
                Empty,
                Text("ng/mL"),
                Number(0.0001),
                Number(0.00025),
                Number(0.0003),
            ],
            vec![Empty, Text("ng/L"), Number(0.1), Number(0.25), Number(0.3)],
        ],
    );
    rows[1][1] = Text("1");
    rows[2][1] = Text("1");

    let run = run_ng_per_ml(&rows);

    let expected =
        vec!["row 2: skipped: in ng/mL; its values are in the ng/L row 3, which is imported"];
    assert_eq!(run.ng_per_ml_rows, expected);
    let error = run
        .error
        .expect("row 3's identification level must stop the run");
    assert!(
        error.contains("has 1 row(s) whose Identification Level is not a number"),
        "{error}"
    );
    assert!(error.contains("\n  row 3: Cell value: \"1\""), "{error}");
    assert!(!error.contains("\n  row 2: "), "{error}");
}
