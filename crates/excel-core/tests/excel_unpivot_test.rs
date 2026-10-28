//! Unpivoting a wide sheet (one column per sample) into one row per sample cell, through the real
//! file pipeline: write an .xlsx, build the validator from it, validate, and export the CSV.

mod common;

use common::Cell::{self, Empty, Error, Number, Text};
use common::{create_simple_mapping, write_test_workbook};
use excel_core::serde_json::json;
use excel_core::{
    DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, ExcelValidatorBuilder, MarkerLimit, Unpivot,
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
            "Units": { "type": ["string", "null"] },
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
    csv_lines: Vec<String>,
}

/// Build with `unpivot`, validate and export `rows` (the first row being the headers).
fn run(name: &str, rows: &[Vec<Cell>], unpivot: Unpivot) -> Result<Unpivoted, String> {
    run_with_schema(name, rows, unpivot, schema())
}

/// [`run`] against `schema` instead of the default dictionary.
fn run_with_schema(
    name: &str,
    rows: &[Vec<Cell>],
    unpivot: Unpivot,
    schema: excel_core::serde_json::Value,
) -> Result<Unpivoted, String> {
    let xlsx = write_test_workbook(name, rows);
    let csv = xlsx.with_extension("csv");

    let result = (|| {
        let mut validator = ExcelValidatorBuilder::new(xlsx.to_str().unwrap(), "Sheet1", schema)
            .unpivot(unpivot)
            .build()
            .map_err(|e| format!("{e:#}"))?;
        let summary = validator
            .unpivot_summary()
            .ok_or("an unpivoted sheet must have a summary")?
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
                create_simple_mapping(vec![
                    "Substance",
                    "Units",
                    "Limit of detection",
                    "Limit of quantification",
                    "Sample Tag",
                    "Concentration",
                ]),
            )
            .map_err(|e| e.to_string())?;
        let csv_lines = std::fs::read_to_string(&csv)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        Ok(Unpivoted { summary, csv_lines })
    })();

    let _ = std::fs::remove_file(&xlsx);
    let _ = std::fs::remove_file(&csv);
    result
}

fn wide_sheet() -> Vec<Vec<Cell>> {
    vec![
        vec![Text("Substance"), Text("Units"), Text("ST01"), Text("ST02")],
        vec![
            Text("Okadaic acid"),
            Text("ng/L"),
            Number(0.88),
            Number(0.4),
        ],
        vec![],
        vec![Text("Azaspiracid-2"), Text("ng/L"), Number(1.25), Empty],
    ]
}

#[test]
fn each_sample_cell_becomes_a_row_with_the_column_header_and_the_cell_value() {
    let unpivoted = run("unpivot-rows", &wide_sheet(), spec()).unwrap();

    // The blank source row produces no rows; the empty cell is an empty number, exported with the
    // usual numeric placeholder
    let expected = vec![
        "Substance,Units,Sample Tag,Concentration",
        "Okadaic acid,ng/L,ST01,0.88",
        "Okadaic acid,ng/L,ST02,0.4",
        "Azaspiracid-2,ng/L,ST01,1.25",
        "Azaspiracid-2,ng/L,ST02,000000000000.000000",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

#[test]
fn the_summary_names_the_kept_and_the_unpivoted_columns_and_the_row_count() {
    let unpivoted = run("unpivot-summary", &wide_sheet(), spec()).unwrap();

    let expected = "Keeping 2 column(s) A-B (Substance … Units); unpivoting 2 column(s) C-D (ST01 … ST02) into Sample Tag / Concentration: 4 row(s).";
    assert_eq!(unpivoted.summary, expected);
}

#[test]
fn text_in_the_values_column_is_a_validation_error_at_its_source_row() {
    let rows = vec![
        vec![Text("Substance"), Text("Units"), Text("ST01"), Text("ST02")],
        vec![
            Text("Okadaic acid"),
            Text("ng/L"),
            Number(0.88),
            Number(0.4),
        ],
        vec![
            Text("Azaspiracid-2"),
            Text("ng/L"),
            Number(1.25),
            Text("<LOD"),
        ],
    ];

    let err = run("unpivot-text-value", &rows, spec()).unwrap_err();

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

    let err = run("unpivot-unknown-field", &wide_sheet(), unpivot)
        .expect_err("an unknown field must be refused");

    assert!(err.contains("headers column \"Sample\""), "{err}");
    assert!(
        err.contains("Candidates: Classification, Concentration, Limit of detection, Limit of quantification, Sample Tag"),
        "{err}"
    );
}

#[test]
fn a_values_column_that_is_already_a_sheet_column_is_refused_with_the_candidates() {
    let unpivot = Unpivot {
        values_column: "Units".to_string(),
        ..spec()
    };

    let err = run("unpivot-sheet-column", &wide_sheet(), unpivot)
        .expect_err("a sheet column must be refused");

    assert!(err.contains("values column \"Units\""), "{err}");
    assert!(
        err.contains("Candidates: Classification, Concentration, Limit of detection, Limit of quantification, Sample Tag"),
        "{err}"
    );
}

#[test]
fn the_same_field_for_headers_and_values_is_refused() {
    let unpivot = Unpivot {
        values_column: "Sample Tag".to_string(),
        ..spec()
    };

    let err = run("unpivot-same-field", &wide_sheet(), unpivot)
        .expect_err("the same field twice must be refused");

    assert!(err.contains("must be different fields"), "{err}");
}

#[test]
fn a_sheet_that_starts_with_a_sample_column_is_refused() {
    let rows = vec![
        vec![Text("ST01"), Text("Substance")],
        vec![Number(0.88), Text("Okadaic acid")],
    ];

    let err = run("unpivot-no-kept", &rows, spec()).expect_err("no kept columns must be refused");

    assert!(err.contains("starts with column A \"ST01\""), "{err}");
}

#[test]
fn a_sheet_without_sample_columns_is_refused() {
    let rows = vec![
        vec![Text("Substance"), Text("Units")],
        vec![Text("Okadaic acid"), Text("ng/L")],
    ];

    let err =
        run("unpivot-no-samples", &rows, spec()).expect_err("no sample columns must be refused");

    assert!(err.contains("no sample columns"), "{err}");
}

/// A dictionary field after the first sample column would otherwise be unpivoted as a sample.
#[test]
fn a_dictionary_field_among_the_sample_columns_is_refused() {
    let rows = vec![
        vec![Text("Substance"), Text("ST01"), Text("Units"), Text("ST02")],
        vec![
            Text("Okadaic acid"),
            Number(0.88),
            Text("ng/L"),
            Number(0.4),
        ],
    ];

    let err = run("unpivot-misplaced-field", &rows, spec())
        .expect_err("a misplaced dictionary field must be refused");

    let expected =
        "column C \"Units\" is a data dictionary field after the first sample column B \"ST01\"";
    assert!(err.contains(expected), "{err}");
}

#[test]
fn a_sample_column_without_a_header_is_refused() {
    let rows = vec![
        vec![Text("Substance"), Text("ST01"), Empty],
        vec![Text("Okadaic acid"), Number(0.88), Number(0.4)],
    ];

    let err = run("unpivot-unnamed-sample", &rows, spec())
        .expect_err("a sample column without a header must be refused");

    assert!(
        err.contains("column C (first value at row 2: \"0.4\")"),
        "{err}"
    );
}

/// An error in a kept cell is one broken cell, reported once, not once per sample column.
#[test]
fn excel_error_cells_are_reported_once_each() {
    let rows = vec![
        vec![Text("Substance"), Text("Units"), Text("ST01"), Text("ST02")],
        vec![
            Text("Okadaic acid"),
            Error("#N/A"),
            Number(0.88),
            Error("#DIV/0!"),
        ],
    ];

    let err =
        run("unpivot-error-cells", &rows, spec()).expect_err("error cells must stop the import");

    assert!(err.contains("contains 2 Excel error value(s)"), "{err}");
    assert!(err.contains("row 2, column 'Units': #N/A"), "{err}");
    assert!(err.contains("row 2, column 'ST02': #DIV/0!"), "{err}");
}

/// Every outcome of a sample cell going to a values field that takes text.
fn marker_sheet(cells: [Cell; 3]) -> Vec<Vec<Cell>> {
    let [st01, st02, st03] = cells;
    vec![
        vec![
            Text("Substance"),
            Text("Units"),
            Text("ST01"),
            Text("ST02"),
            Text("ST03"),
        ],
        vec![Text("Okadaic acid"), Text("ng/L"), st01, st02, st03],
    ]
}

#[test]
fn a_text_values_field_takes_numbers_markers_and_blanks_as_written() {
    let mut rows = marker_sheet([Number(0.880999866251062), Text("NS "), Text("<LOD")]);
    rows.push(vec![
        Text("Azaspiracid-2"),
        Text("ng/L"),
        Text("<LOQ"),
        Empty,
        Text("0.5"),
    ]);

    let unpivoted = run_with_schema("unpivot-markers", &rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Units,Sample Tag,Concentration",
        "Okadaic acid,ng/L,ST01,0.880999866251062",
        "Okadaic acid,ng/L,ST02,NS",
        "Okadaic acid,ng/L,ST03,<LOD",
        "Azaspiracid-2,ng/L,ST01,<LOQ",
        "Azaspiracid-2,ng/L,ST02,",
        "Azaspiracid-2,ng/L,ST03,0.5",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
}

#[test]
fn other_text_in_a_text_values_field_is_a_validation_error_naming_the_markers() {
    let rows = marker_sheet([Text("<LDO"), Text("<lod"), Text("NaN")]);

    let err = run_with_schema("unpivot-bad-marker", &rows, spec(), text_schema()).unwrap_err();

    for value in ["<LDO", "<lod", "NaN"] {
        let expected = format!(
            "row[2]./Concentration: '{value}' is not a number or one of the markers <LOD, <LOQ, NS"
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

    let err = run_with_schema("unpivot-own-markers", &rows, unpivot, text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LOD' is not a number or one of the markers <MDL";
    assert!(err.contains(expected), "{err}");
    assert!(!err.contains("'<MDL'"), "{err}");
}

#[test]
fn the_summary_of_a_text_values_field_names_the_markers() {
    let unpivoted = run_with_schema(
        "unpivot-markers-summary",
        &wide_sheet(),
        spec(),
        text_schema(),
    )
    .unwrap();

    let expected = "Keeping 2 column(s) A-B (Substance … Units); unpivoting 2 column(s) C-D (ST01 … ST02) into Sample Tag / Concentration: 4 row(s). Concentration takes numbers or <LOD, <LOQ, NS.";
    assert_eq!(unpivoted.summary, expected);
}

/// A row with both limits and three sample cells: `cells` for an LOD of 0.1 and an LOQ of `loq`.
fn limits_sheet(loq: Cell, cells: [Cell; 3]) -> Vec<Vec<Cell>> {
    let [st01, st02, st03] = cells;
    vec![
        vec![
            Text("Substance"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
            Text("ST02"),
            Text("ST03"),
        ],
        vec![Text("Okadaic acid"), Number(0.1), loq, st01, st02, st03],
    ]
}

#[test]
fn a_marker_followed_by_the_rows_limit_is_written_as_the_marker() {
    let rows = limits_sheet(
        Number(0.25),
        [Text("<LOQ:0.25"), Text("<LOD : 0.100"), Number(0.3)],
    );

    let unpivoted = run_with_schema("unpivot-limit-match", &rows, spec(), text_schema()).unwrap();

    let expected = vec![
        "Substance,Limit of detection,Limit of quantification,Sample Tag,Concentration",
        "Okadaic acid,0.1,0.25,ST01,<LOQ",
        "Okadaic acid,0.1,0.25,ST02,<LOD",
        "Okadaic acid,0.1,0.25,ST03,0.3",
    ];
    assert_eq!(unpivoted.csv_lines, expected);
    let expected_summary = "Concentration takes numbers or <LOD, <LOQ, NS. 2 value(s) written as their marker alone, the number after it being the row's limit: <LOD (Limit of detection), <LOQ (Limit of quantification).";
    assert!(
        unpivoted.summary.ends_with(expected_summary),
        "{}",
        unpivoted.summary
    );
}

#[test]
fn without_such_values_the_summary_does_not_mention_limits() {
    let rows = limits_sheet(Number(0.25), [Text("<LOQ"), Text("NS"), Number(0.3)]);

    let unpivoted = run_with_schema("unpivot-limit-none", &rows, spec(), text_schema()).unwrap();

    assert!(
        unpivoted
            .summary
            .ends_with("Concentration takes numbers or <LOD, <LOQ, NS."),
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

    let err = run_with_schema("unpivot-limit-mismatch", &rows, spec(), text_schema()).unwrap_err();

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

    let err = run_with_schema("unpivot-limit-blank", &rows, spec(), text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LOQ:0.25' has a number, but the row has no Limit of quantification";
    assert!(err.contains(expected), "{err}");
}

/// Only markers given a limit take a number after them.
#[test]
fn a_marker_without_a_limit_followed_by_a_number_is_a_validation_error() {
    let rows = limits_sheet(Number(0.25), [Text("NS:0.25"), Text("NS"), Number(0.3)]);

    let err = run_with_schema("unpivot-limit-ns", &rows, spec(), text_schema()).unwrap_err();

    let expected =
        "row[2]./Concentration: 'NS:0.25' is not a number or one of the markers <LOD, <LOQ, NS";
    assert!(err.contains(expected), "{err}");
}

#[test]
fn a_limit_that_is_not_a_dictionary_field_is_refused() {
    let unpivot = Unpivot {
        marker_limits: vec!["<LOQ=Limit of quant".parse().unwrap()],
        ..spec()
    };
    let rows = limits_sheet(Number(0.25), [Text("<LOQ"), Text("NS"), Number(0.3)]);

    let err = run_with_schema("unpivot-limit-unknown", &rows, unpivot, text_schema()).unwrap_err();

    assert!(err.contains("\"Limit of quant\""), "{err}");
    assert!(err.contains("not a data dictionary field"), "{err}");
}

/// The default limits do not stand in the way of other markers, and do not apply without theirs.
#[test]
fn a_limit_for_a_marker_that_is_not_a_values_marker_does_not_apply() {
    let unpivot = Unpivot {
        values_markers: vec!["NS".to_string()],
        ..spec()
    };
    let rows = limits_sheet(Number(0.25), [Text("NS"), Text("<LOQ:0.25"), Number(0.3)]);

    let err = run_with_schema("unpivot-limit-marker", &rows, unpivot, text_schema()).unwrap_err();

    let expected = "row[2]./Concentration: '<LOQ:0.25' is not a number or one of the markers NS";
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
        "unpivot-limit-number",
        &rows,
        unpivot,
        schema_with_concentration(json!({ "type": ["number", "null"] })),
    )
    .unwrap();

    let expected = "Okadaic acid,0.1,0.25,ST03,0.3";
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
fn nd_among_the_given_markers_is_refused_pointing_to_ns() {
    for nd in ["ND", " nd "] {
        let unpivot = Unpivot {
            values_markers: vec!["NS".to_string(), nd.to_string()],
            ..spec()
        };
        let rows = marker_sheet([Text("NS"), Text("NS"), Number(0.4)]);

        let err = run_with_schema("unpivot-nd-marker", &rows, unpivot, text_schema()).unwrap_err();

        let expected = format!(
            "\"{}\" cannot be a values marker: ND maps to NS. Use NS, and change ND to NS in the source file, for consistency.",
            nd.trim()
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
}

#[test]
fn nd_in_a_sample_cell_is_a_validation_error_pointing_to_ns() {
    let rows = marker_sheet([Text("ND"), Text("nd"), Text("NS")]);

    let err = run_with_schema("unpivot-nd-cell", &rows, spec(), text_schema()).unwrap_err();

    for value in ["ND", "nd"] {
        let expected = format!(
            "row[2]./Concentration: '{value}' is not allowed: ND maps to NS. Change it to NS in the source file, for consistency."
        );
        assert!(err.contains(&expected), "{expected}\n{err}");
    }
    assert!(!err.contains("'NS'"), "{err}");
}
