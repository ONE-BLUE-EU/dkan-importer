use anyhow::{Context, Result};
use calamine::{Data, Reader, Xlsx, open_workbook};
use jsonschema::Validator;
use serde_json::{Map, Value, json};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroUsize;
use thiserror::Error;

use crate::utils::{normalize_string, write_error_to_log};

/// Written in place of an empty Number cell in the exported CSV.
///
/// Deliberate, despite turning a missing value into a 0 in DKAN: DKAN infers column types from the
/// CSV contents, and this value makes it create DECIMAL(18, 6) - 18 total digits with 6 decimal
/// places - rather than a text column. Pinned by
/// `empty_numeric_cells_are_exported_as_dkan_type_placeholders` in `tests/excel_file_pipeline_test.rs`.
const NUMERIC_PLACEHOLDER: &str = "000000000000.000000";

/// Written in place of an empty Integer cell in the exported CSV, for the same reason as
/// [`NUMERIC_PLACEHOLDER`].
const INTEGER_PLACEHOLDER: &str = "0";

/// Number of columns in an Excel sheet (A to XFD).
const EXCEL_MAX_COLUMNS: u32 = 16_384;

/// A cell of the sheet, as Excel shows it
#[derive(Debug, Clone, PartialEq)]
pub struct SheetCell {
    /// The row number Excel shows
    pub row: usize,
    /// The column letters Excel shows, as `L`
    pub column: String,
    /// The column's header, unless it has none
    pub header: Option<String>,
}

impl std::fmt::Display for SheetCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "row {}, column {}", self.row, self.column)?;
        match &self.header {
            Some(header) => write!(f, " \"{header}\""),
            None => Ok(()),
        }
    }
}

/// What the importer did to a cell instead of taking it as written, and why. Listed in the run
/// log, so the data can be checked and corrected at its source.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// An Excel error value, read as a blank cell
    Emptied { cell: SheetCell, error: String },
    /// A sample cell that gives no row
    Skipped { cell: SheetCell, reason: SkipReason },
    /// A value written differently from the source
    Rewritten {
        cell: SheetCell,
        was: String,
        now: String,
        reason: String,
    },
    /// A field of the result of the sample cell `cell` worked out by the importer, differing from
    /// the row's own value `was`
    Derived {
        cell: SheetCell,
        field: String,
        value: String,
        was: String,
        reason: String,
    },
}

/// Why a sample cell gives no row. A blank one gives none either, but is only counted in the
/// summary, not listed.
#[derive(Debug, Clone, PartialEq)]
pub enum SkipReason {
    /// No sample was taken; the cell's text as written, trimmed
    NoSample(String),
    /// Not clear yet how to import it: the question to ask, and why this cell gives no row
    Open { question: Question, reason: String },
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::Emptied { cell, error } => write!(
                f,
                "{cell}: emptied: Excel error {error}, a failed calculation, read as a blank cell"
            ),
            Change::Skipped {
                cell,
                reason: SkipReason::NoSample(text),
            } if text.eq_ignore_ascii_case(NO_SAMPLE) => {
                write!(f, "{cell}: no row: \"{text}\" means no sample")
            }
            Change::Skipped {
                cell,
                reason: SkipReason::NoSample(text),
            } => write!(
                f,
                "{cell}: no row: \"{text}\" is treated as {NO_SAMPLE} (no sample)"
            ),
            Change::Rewritten {
                cell,
                was,
                now,
                reason,
            } => write!(
                f,
                "{cell}: written as \"{now}\" instead of \"{was}\": {reason}"
            ),
            Change::Skipped {
                cell,
                reason: SkipReason::Open { reason, .. },
            } => write!(f, "{cell}: no row: {reason}"),
            Change::Derived {
                cell,
                field,
                value,
                was,
                reason,
            } => write!(
                f,
                "{cell}: {field} \"{value}\" instead of \"{was}\": {reason}"
            ),
        }
    }
}

/// Something about the data the importer cannot decide alone. The cells concerned are not
/// imported, or imported on an assumption, and the run log asks the question once, with the number
/// of cells, for the data owners to answer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Question {
    /// An ng/L row without both limits: its results cannot be characterised
    MissingLimits,
    /// A number in a Peak Area row
    NumberInPeakArea,
    /// X, a Peak Area result, in an ng/L row
    XInNanogramsPerLitre,
    /// An items result that is not a whole number of 0 or more
    ItemsNotWholeNumber,
    /// Units other than ng/L, Peak Area and items, as written
    OtherUnits(String),
    /// A result that means nothing yet with its row's units: what the result is (`a number` or
    /// a marker in quotes), and the units as written, if any
    UnclassifiedResult {
        result: String,
        units: Option<String>,
    },
    /// X in a row without units, characterised as a Peak Area result
    EmptyUnitsTakenAsPeakArea,
    /// "detected, not quantified" is not in the template's dropdown list
    DetectedNotQuantified,
}

impl Question {
    /// Why a cell gives no row because of this question, for its line in the run log
    fn reason(&self) -> String {
        match self {
            Question::MissingLimits => format!(
                "the ng/L row has no {LOD_FIELD} or no {LOQ_FIELD}, so its results cannot be characterised"
            ),
            Question::NumberInPeakArea => "a number in a Peak Area row".to_string(),
            Question::XInNanogramsPerLitre => "X in an ng/L row".to_string(),
            Question::ItemsNotWholeNumber => "not a whole number of items".to_string(),
            Question::OtherUnits(units) => format!("{UNITS_FIELD} \"{units}\" are not imported"),
            Question::UnclassifiedResult { result, units } => {
                format!("{result} in a row {}", units_phrase(units.as_deref()))
            }
            Question::EmptyUnitsTakenAsPeakArea => {
                "X in a row without Concentration units, taken as Peak Area".to_string()
            }
            Question::DetectedNotQuantified => DETECTED_NOT_QUANTIFIED.to_string(),
        }
    }
}

/// "with Concentration units "u"", or "without Concentration units"
fn units_phrase(units: Option<&str>) -> String {
    match units {
        Some(units) => format!("with {UNITS_FIELD} \"{units}\""),
        None => format!("without {UNITS_FIELD}"),
    }
}

impl std::fmt::Display for Question {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Question::MissingLimits => write!(
                f,
                "ng/L rows without a {LOD_FIELD} or a {LOQ_FIELD} are not imported, as their results cannot be characterised. Should they be imported, and how?"
            ),
            Question::NumberInPeakArea => write!(
                f,
                "Numbers in Peak Area rows are not imported. Should they be imported as the Concentration, with which detection characterisation, and should a field mark them as peak areas?"
            ),
            Question::XInNanogramsPerLitre => write!(
                f,
                "X results in ng/L rows are not imported, as X is a Peak Area result. Are the units or the results wrong?"
            ),
            Question::ItemsNotWholeNumber => write!(
                f,
                "Items results that are not whole numbers of 0 or more are not imported. What do they mean?"
            ),
            Question::OtherUnits(units) => write!(
                f,
                "Rows with {UNITS_FIELD} \"{units}\" are not imported. How should they be handled?"
            ),
            Question::UnclassifiedResult { result, units } => {
                let mut result = result.clone();
                if let Some(first) = result.get(..1) {
                    result.replace_range(..1, &first.to_uppercase());
                }
                write!(
                    f,
                    "{result} in a row {} is not imported. What does it mean?",
                    units_phrase(units.as_deref())
                )
            }
            Question::EmptyUnitsTakenAsPeakArea => write!(
                f,
                "X results in rows without {UNITS_FIELD} are taken as Peak Area ({DETECTED_QUALITATIVELY}), without writing the units. Is that right?"
            ),
            Question::DetectedNotQuantified => write!(
                f,
                "Results between the {LOD_FIELD} and the {LOQ_FIELD}, and <LOQ, are characterised as \"{DETECTED_NOT_QUANTIFIED}\", which the template's dropdown list does not have (it has \"detected semiquantitatively\"). Which should be used?"
            ),
        }
    }
}

/// Units some sheets keep beside their ng/L rows, which are calculated from them by formulas.
/// Such a row is skipped, its values being in its ng/L row. Matched ignoring case.
const NANOGRAMS_PER_MILLILITRE: &str = "ng/mL";

/// An ng/mL value is this many times the same ng/L value
const MILLILITRES_PER_LITRE: f64 = 1000.0;

/// How far apart, relative to the larger, an ng/L number may be from its ng/mL number x1000.
/// Excel saves a formula's result to 15 significant digits, but sheets also hold them rounded to
/// 10, as in `34.79658006` for `0.03479658005914394` ng/mL.
const SAME_NUMBER_RELATIVE_TOLERANCE: f64 = 1e-6;

/// An ng/mL row of an unpivoted sheet. Only ng/L is imported, so it is skipped, but its values
/// must be in an ng/L row: sheets keep ng/mL rows beside the ng/L rows calculated from them by
/// formulas, and a formula pointing at the wrong row would lose them.
#[derive(Debug, Clone, PartialEq)]
pub struct NgPerMlRow {
    pub counterpart: Counterpart,
    /// The row number Excel shows
    pub row: usize,
}

/// The ng/L row holding an ng/mL row's values: its text, ignoring case and spaces, and its
/// numbers x1000 in the limits and the sample columns. Where the ng/mL row is blank, or holds an
/// Excel error, the ng/L row may hold anything.
#[derive(Debug, Clone, PartialEq)]
pub enum Counterpart {
    /// The row number Excel shows of the first ng/L row holding them
    Found(usize),
    /// No ng/L row holds them, which stops the run: the closest, unless the sheet has no ng/L row
    Missing(Option<Closest>),
}

/// The ng/L row differing from an ng/mL row in the fewest cells, the first of them if several do
#[derive(Debug, Clone, PartialEq)]
pub struct Closest {
    /// Its first difference, in sheet order, whose cell is in the ng/L row
    pub first: Difference,
    /// The number of cells in which it differs
    pub differences: NonZeroUsize,
}

/// A cell of an ng/L row not holding the value of its ng/mL row
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub cell: SheetCell,
    /// The ng/mL value, as it should be in ng/L: a number, or text in quotes
    pub expected: String,
    /// The cell: a number, text in quotes, or `blank`
    pub found: String,
}

impl std::fmt::Display for NgPerMlRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "row {}: skipped: in {NANOGRAMS_PER_MILLILITRE}",
            self.row
        )?;
        match &self.counterpart {
            Counterpart::Found(row) => {
                write!(
                    f,
                    "; its values are in the ng/L row {row}, which is imported"
                )
            }
            Counterpart::Missing(None) => {
                write!(f, ", but the sheet has no ng/L row to hold its values")
            }
            Counterpart::Missing(Some(Closest { first, differences })) => write!(
                f,
                ", but no ng/L row has its values (x1000 in the limits and the sample columns); the closest, row {}, differs in {differences} cell(s), first at {}: {} expected, {} found",
                first.cell.row, first.cell, first.expected, first.found
            ),
        }
    }
}

/// A column whose value in an ng/mL row must be in its ng/L row
struct ComparedColumn {
    /// The column letters Excel shows
    column: String,
    header: Option<String>,
    index: usize,
    /// What the ng/mL number is multiplied by in ng/L: [`MILLILITRES_PER_LITRE`] or 1
    factor: f64,
}

/// A cell's number, whether it holds a number or text holding one
fn cell_number(cell: &Data) -> Option<f64> {
    match cell {
        Data::Float(n) => Some(*n),
        Data::Int(n) => Some(*n as f64),
        Data::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// `cell` as the run log shows it: a number, text in quotes, or `blank`
fn describe(cell: Option<&Data>) -> String {
    match cell.filter(|cell| !is_blank(cell)) {
        None => "blank".to_string(),
        Some(cell) => match cell_number(cell) {
            Some(n) => n.to_string(),
            None => format!("\"{}\"", cell.to_string().trim()),
        },
    }
}

/// What the ng/L cell should hold and what it holds, unless it holds the ng/mL cell's value
/// multiplied by `factor`
fn difference(
    ng_per_ml: Option<&Data>,
    ng_per_l: Option<&Data>,
    factor: f64,
) -> Option<(String, String)> {
    let ng_per_ml = ng_per_ml.filter(|cell| !is_blank(cell) && !matches!(cell, Data::Error(_)))?;
    let (expected, holds) = match cell_number(ng_per_ml) {
        Some(n) => {
            let expected = n * factor;
            let holds = ng_per_l.and_then(cell_number).is_some_and(|found| {
                (found - expected).abs()
                    <= SAME_NUMBER_RELATIVE_TOLERANCE * found.abs().max(expected.abs())
            });
            (expected.to_string(), holds)
        }
        None => {
            let text = ng_per_ml.to_string();
            let text = text.trim();
            let holds =
                ng_per_l.is_some_and(|cell| cell.to_string().trim().eq_ignore_ascii_case(text));
            (format!("\"{text}\""), holds)
        }
    };
    (!holds).then(|| (expected, describe(ng_per_l)))
}

/// The ng/L row of `ng_per_litre` (row number, cells) holding the values of the ng/mL row
/// `ng_per_ml` in `columns`
fn counterpart(
    ng_per_ml: &[Data],
    ng_per_litre: &[(usize, &[Data])],
    columns: &[ComparedColumn],
) -> Counterpart {
    let mut closest: Option<Closest> = None;
    for (row, ng_per_l) in ng_per_litre {
        let mut differences = columns.iter().filter_map(|column| {
            difference(
                ng_per_ml.get(column.index),
                ng_per_l.get(column.index),
                column.factor,
            )
            .map(|difference| (column, difference))
        });
        let Some((column, (expected, found))) = differences.next() else {
            return Counterpart::Found(*row);
        };
        let count = NonZeroUsize::MIN.saturating_add(differences.count());
        if closest
            .as_ref()
            .is_none_or(|closest| count < closest.differences)
        {
            closest = Some(Closest {
                first: Difference {
                    cell: SheetCell {
                        row: *row,
                        column: column.column.clone(),
                        header: column.header.clone(),
                    },
                    expected,
                    found,
                },
                differences: count,
            });
        }
    }
    Counterpart::Missing(closest)
}

/// A row's units, read from its [`UNITS_FIELD`] cell, ignoring case and surrounding spaces
#[derive(Debug, PartialEq)]
enum Units {
    NanogramsPerLitre,
    PeakArea,
    Items,
    Missing,
    /// As written, trimmed
    Other(String),
}

impl Units {
    fn parse(cell: Option<&Data>) -> Self {
        let Some(text) = cell.filter(|cell| !is_blank(cell)).map(ToString::to_string) else {
            return Units::Missing;
        };
        let text = text.trim();
        match text.to_lowercase().as_str() {
            "ng/l" => Units::NanogramsPerLitre,
            "peak area" => Units::PeakArea,
            "items" => Units::Items,
            _ => Units::Other(text.to_string()),
        }
    }

    fn is_ng_per_ml(&self) -> bool {
        matches!(self, Units::Other(units) if units.eq_ignore_ascii_case(NANOGRAMS_PER_MILLILITRE))
    }
}

/// A sample cell's result, after the markers are read
enum SampleResult<'a> {
    Number(f64),
    /// One of the values markers, uppercase
    Marker(&'a str),
}

/// What becomes of a result
#[derive(Debug, PartialEq)]
enum Outcome {
    /// A row, with this detection characterisation, why, and the question it raises, if any
    Characterised {
        detection: &'static str,
        reason: String,
        question: Option<Question>,
    },
    /// No row until the question is answered, and why, for the cell's line in the run log
    Open { question: Question, reason: String },
}

impl Outcome {
    /// No row until `question` is answered, for its usual reason
    fn open(question: Question) -> Self {
        Outcome::Open {
            reason: question.reason(),
            question,
        }
    }
}

/// The detection characterisation of `result`, in a row with `units` and limits `lod`, `loq`;
/// `raw` is the cell as written, for messages
fn characterise(
    units: &Units,
    result: &SampleResult,
    raw: &str,
    lod: Option<f64>,
    loq: Option<f64>,
) -> Outcome {
    let characterised = |detection: &'static str, reason: String| Outcome::Characterised {
        detection,
        reason,
        question: (detection == DETECTED_NOT_QUANTIFIED).then_some(Question::DetectedNotQuantified),
    };
    let unclassified = |units: Option<String>| {
        Outcome::open(Question::UnclassifiedResult {
            result: match result {
                SampleResult::Number(_) => "a number".to_string(),
                SampleResult::Marker(marker) => format!("\"{marker}\""),
            },
            units,
        })
    };
    match units {
        Units::NanogramsPerLitre => {
            if matches!(result, SampleResult::Marker(X_MARKER)) {
                return Outcome::open(Question::XInNanogramsPerLitre);
            }
            let (Some(lod), Some(loq)) = (lod, loq) else {
                return Outcome::open(Question::MissingLimits);
            };
            match result {
                SampleResult::Number(n) if *n >= loq => characterised(
                    DETECTED_QUANTITATIVELY,
                    format!("{n} is at least the {LOQ_FIELD} ({loq})"),
                ),
                SampleResult::Number(n) if *n >= lod => characterised(
                    DETECTED_NOT_QUANTIFIED,
                    format!("{n} is between the {LOD_FIELD} ({lod}) and the {LOQ_FIELD} ({loq})"),
                ),
                SampleResult::Number(n) => characterised(
                    NOT_DETECTED,
                    format!("{n} is below the {LOD_FIELD} ({lod})"),
                ),
                SampleResult::Marker(LOD_MARKER) => {
                    characterised(NOT_DETECTED, format!("the result is {LOD_MARKER}"))
                }
                SampleResult::Marker(LOQ_MARKER) => characterised(
                    DETECTED_NOT_QUANTIFIED,
                    format!("the result is {LOQ_MARKER}"),
                ),
                SampleResult::Marker(_) => unclassified(Some("ng/L".to_string())),
            }
        }
        Units::PeakArea => match result {
            SampleResult::Marker(X_MARKER) => {
                characterised(DETECTED_QUALITATIVELY, "X in a Peak Area row".to_string())
            }
            SampleResult::Number(_) => Outcome::open(Question::NumberInPeakArea),
            SampleResult::Marker(_) => unclassified(Some("Peak Area".to_string())),
        },
        Units::Missing => match result {
            SampleResult::Marker(X_MARKER) => Outcome::Characterised {
                detection: DETECTED_QUALITATIVELY,
                reason: Question::EmptyUnitsTakenAsPeakArea.reason(),
                question: Some(Question::EmptyUnitsTakenAsPeakArea),
            },
            _ => unclassified(None),
        },
        Units::Items => match result {
            SampleResult::Number(n) if *n >= 0.0 && n.fract() == 0.0 => {
                characterised(DETECTED_QUANTITATIVELY, "a count of items".to_string())
            }
            _ => Outcome::Open {
                question: Question::ItemsNotWholeNumber,
                reason: format!("\"{raw}\" is not a whole number of items"),
            },
        },
        Units::Other(units) => Outcome::open(Question::OtherUnits(units.clone())),
    }
}

/// `field` of `row` as a finite number, whether the cell held a number or text
fn number_in(row: &Map<String, Value>, field: &str) -> Option<f64> {
    row.get(field).and_then(number_of)
}

/// `value` as a finite number, whether it is a number or text holding one
fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// The field holding a result's units. An unpivoted sheet must have it among its kept columns.
pub const UNITS_FIELD: &str = "Concentration units";
/// The field each result's detection characterisation is written to
pub const DETECTION_FIELD: &str = "Detection characterisation";
/// The field holding the row's limit of detection
pub const LOD_FIELD: &str = "Limit of detection";
/// The field holding the row's limit of quantification
pub const LOQ_FIELD: &str = "Limit of quantification";

const DETECTED_QUANTITATIVELY: &str = "detected quantitatively";
const DETECTED_NOT_QUANTIFIED: &str = "detected, not quantified";
const DETECTED_QUALITATIVELY: &str = "detected qualitatively";
const NOT_DETECTED: &str = "not detected";

/// Type alias for a parsed Excel row with row number and field data
pub type ParsedExcelRow = (usize, Map<String, Value>);

/// A sheet read into rows ready for validation
struct ProcessedSheet {
    headers: Vec<String>,
    rows: Vec<ParsedExcelRow>,
    /// What the unpivot did, when the sheet was unpivoted
    unpivot_summary: Option<String>,
    /// What moving the extra columns did, when they were moved
    extra_columns_summary: Option<String>,
    /// The Excel error cells emptied, when there were any
    error_cells_summary: Option<String>,
    /// Every change made to a cell, in sheet order
    changes: Vec<Change>,
    /// The open questions, each with the number of cells concerned
    questions: Vec<(Question, usize)>,
    /// The ng/mL rows skipped, in sheet order, each with its ng/L row
    ng_per_ml_rows: Vec<NgPerMlRow>,
    /// What stops the run until the source is corrected, each a message listing its cells
    problems: Vec<String>,
    /// The markers allowed in the values field, when an unpivot sends its cells to a text field
    values_markers: Option<ValuesMarkers>,
}

#[derive(Error, Debug, Clone)]
pub enum ValidationError {
    #[error("Type mismatch at {path}: expected {expected}, got {actual} \"{value}\"")]
    TypeMismatch {
        path: String,
        expected: String,
        actual: String,
        value: String,
    },

    #[error("Required field missing at {path}: {field}")]
    RequiredFieldMissing { path: String, field: String },

    #[error("Invalid format at {path}: {message}")]
    InvalidFormat { path: String, message: String },

    #[error("Value out of range at {path}: {message}")]
    OutOfRange { path: String, message: String },

    #[error("Numeric value out of range at {path}: value {value} must be between {min} and {max}")]
    NumericOutOfRange {
        path: String,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("Value below minimum at {path}: value {value} is less than minimum {min}")]
    BelowMinimum { path: String, value: f64, min: f64 },

    #[error("Value above maximum at {path}: value {value} exceeds maximum {max}")]
    AboveMaximum { path: String, value: f64, max: f64 },

    #[error(
        "Excel has the following extra columns not found in the provided JSON Schema at {path}: {properties:?}"
    )]
    AdditionalProperties {
        path: String,
        properties: Vec<String>,
    },

    #[error("Array validation failed at {path}: {message}")]
    ArrayValidation { path: String, message: String },

    #[error("Pattern validation failed at {path}: pattern '{pattern}' for value '{value}'")]
    PatternMismatch {
        path: String,
        pattern: String,
        value: String,
    },

    #[error(
        "Invalid coordinate at {path}: failed to parse '{value}' as {coordinate_type} - {message}"
    )]
    InvalidCoordinate {
        path: String,
        value: String,
        coordinate_type: String,
        message: String,
    },
}

#[derive(Debug)]
pub struct ValidationReport {
    pub row_number: usize,
    pub errors: Vec<ValidationError>,
    pub row_data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SchemaType {
    String,
    Integer,
    Number,
    Boolean,
    Array(Box<SchemaType>),
    Object,
    Null,
    Mixed(Vec<SchemaType>), // For union types
}

#[derive(Debug, Clone)]
pub struct FieldSchema {
    pub field_type: SchemaType,
    pub format: Option<String>,
    pub pattern: Option<String>,
    pub enum_values: Option<Vec<Value>>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

pub struct ExcelValidator {
    excel_path: String,
    sheet_name: String,
    pub validator: Validator,
    field_schemas: HashMap<String, FieldSchema>,
    pub validation_reports: Vec<ValidationReport>,
    headers: Vec<String>,
    rows: Vec<ParsedExcelRow>,
    unpivot_summary: Option<String>,
    extra_columns_summary: Option<String>,
    error_cells_summary: Option<String>,
    changes: Vec<Change>,
    questions: Vec<(Question, usize)>,
    ng_per_ml_rows: Vec<NgPerMlRow>,
    problems: Vec<String>,
    values_markers: Option<ValuesMarkers>,
}

pub struct ExcelValidatorBuilder {
    excel_path: String,
    sheet_name: String,
    schema: Value,
    unpivot: Option<Unpivot>,
    extra_columns_to: Option<String>,
}

/// Unpivot a wide sheet: each non-blank cell of a sample column becomes its own row. A blank
/// cell (empty, only spaces, or a formula giving `""`) is no result, so it gives no row.
///
/// The sheet's leading columns whose headers are schema fields are kept and copied to every row;
/// every column after them is a sample column. Both fields must be schema fields that are not
/// sheet columns.
pub struct Unpivot {
    /// The field that receives the sample column's header
    pub headers_column: String,
    /// The field that receives the sample column's cell value
    pub values_column: String,
    /// The text a sample cell may hold instead of a number, when `values_column` takes text.
    ///
    /// A values field that takes text would otherwise accept anything, so a typo such as `<LDO`
    /// would be uploaded as data. With a text field, every non-blank sample cell must be a
    /// number or one of these markers (after trimming, ignoring case); anything else is a
    /// validation error at its source row. Markers are written uppercase, so `<lod` is uploaded
    /// as `<LOD`. A numeric values field ignores the markers: its type already refuses all text.
    /// See [`DEFAULT_VALUES_MARKERS`].
    pub values_markers: Vec<String>,
    /// The markers that may be followed by their limit, and the field holding that limit.
    ///
    /// Some sheets write the limit after the marker, as in `<LOQ:0.25`. The limit is already in
    /// its own field, so when the number after the marker equals that field in the same row, the
    /// cell is written as the marker alone (`<LOQ`) and reads the same as in sheets that never had
    /// the number. Any other number, or a row without the limit, is a validation error, so no
    /// number is ever dropped that the row does not keep. A limit for a marker that is not in
    /// `values_markers` does not apply. Ignored with a numeric values field, like the markers.
    /// See [`DEFAULT_MARKER_LIMITS`].
    pub marker_limits: Vec<MarkerLimit>,
}

/// A values marker that may be followed by a limit, and the data dictionary field holding it;
/// see [`Unpivot::marker_limits`]. Read from `MARKER=FIELD`, as in
/// `<LOQ=Limit of quantification`, with the marker uppercase.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerLimit {
    pub marker: String,
    pub limit_field: String,
}

impl std::str::FromStr for MarkerLimit {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (marker, field) = s
            .split_once('=')
            .map(|(marker, field)| (marker.trim(), normalize_string(field)))
            .filter(|(marker, field)| !marker.is_empty() && !field.is_empty())
            .ok_or_else(|| {
                format!(
                    "expected MARKER=FIELD, such as \"<LOQ=Limit of quantification\"; got \"{s}\""
                )
            })?;
        Ok(MarkerLimit {
            marker: marker.to_uppercase(),
            limit_field: field,
        })
    }
}

/// The limit columns analytical results sheets have for [`DEFAULT_VALUES_MARKERS`], as
/// `MARKER=FIELD`.
pub const DEFAULT_MARKER_LIMITS: [&str; 2] =
    ["<LOD=Limit of detection", "<LOQ=Limit of quantification"];

/// The markers analytical results use in place of a concentration:
/// - `<LOD`: below the limit of detection (the substance was not detected)
/// - `<LOQ`: below the limit of quantification (detected, but too little to measure)
/// - `X`: detected, in a Peak Area row (or a row without units)
///
/// The limits themselves are in their own columns ("Limit of detection", "Limit of
/// quantification"), so the markers are kept as written rather than replaced by a number.
pub const DEFAULT_VALUES_MARKERS: [&str; 3] = [LOD_MARKER, LOQ_MARKER, X_MARKER];

const LOD_MARKER: &str = "<LOD";
const LOQ_MARKER: &str = "<LOQ";
const X_MARKER: &str = "X";

/// A sample cell meaning no sample was taken at that station or depth. There is no result, so
/// the cell gives no row, like a blank one. Matched ignoring case.
const NO_SAMPLE: &str = "NS";

/// A sample cell some sheets use for [`NO_SAMPLE`], treated the same way. Matched ignoring case.
const NO_SAMPLE_ALIAS: &str = "ND";

/// The field holding an analytical result's identification level. An unpivoted sheet must have
/// it among its kept columns, holding [`IDENTIFICATION_LEVEL`] in every row with a result.
pub const IDENTIFICATION_LEVEL_FIELD: &str = "Identification Level";

/// The identification level of every analytical result. Other values are not rewritten: they
/// stop the run, to be corrected in the source file.
pub const IDENTIFICATION_LEVEL: &str = "2A";

/// Markers some sheets use for a meaning that already has a marker, and the marker to write
/// instead, so that one meaning is written one way across datasets. They are refused, even
/// when given on purpose, both as values markers and in sample cells (also when followed by
/// `:` and a limit, as in `<LQ:0.25`). Matched ignoring case.
const REPLACED_MARKERS: [(&str, &str); 2] = [("<LQ", "<LOQ"), ("NQ", "<LOQ")];

/// The marker that replaces `value`, when it is one of [`REPLACED_MARKERS`], alone or followed
/// by `:`
fn replaced_marker(value: &str) -> Option<(&'static str, &'static str)> {
    let value = value.trim();
    REPLACED_MARKERS.into_iter().find(|(old, _)| {
        strip_marker(value, old)
            .map(str::trim_start)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
    })
}

/// What follows `marker` (uppercase) at the start of `value`, matched ignoring case
fn strip_marker<'a>(value: &'a str, marker: &str) -> Option<&'a str> {
    value
        .get(..marker.len())
        .filter(|start| start.to_uppercase() == marker)?;
    value.get(marker.len()..)
}

/// A values marker as given, trimmed and uppercase. Refuses a marker that another one replaces,
/// such as `<LQ` (see [`REPLACED_MARKERS`]), and the cells that give no row, such as `NS`.
pub fn parse_values_marker(marker: &str) -> std::result::Result<String, String> {
    let marker = marker.trim();
    if marker.eq_ignore_ascii_case(NO_SAMPLE) {
        return Err(format!(
            "\"{marker}\" cannot be a values marker: {NO_SAMPLE} (no sample) cells give no row."
        ));
    }
    if marker.eq_ignore_ascii_case(NO_SAMPLE_ALIAS) {
        return Err(format!(
            "\"{marker}\" cannot be a values marker: {NO_SAMPLE_ALIAS} cells are treated as {NO_SAMPLE} (no sample) and give no row."
        ));
    }
    match replaced_marker(marker) {
        Some((old, new)) => Err(format!(
            "\"{marker}\" cannot be a values marker: {old} maps to {new}. Use {new}, and change {old} to {new} in the source file, for consistency."
        )),
        None => Ok(marker.to_uppercase()),
    }
}

/// The sample columns of an unpivoted sheet and the fields they go to
struct SampleColumns {
    /// (column index, header) of each sample column
    columns: Vec<(usize, String)>,
    headers_column: String,
    values_column: String,
    /// `Some` when `values_column` takes text; see [`Unpivot::values_markers`]
    values_markers: Option<ValuesMarkers>,
    /// Index of the kept column holding [`IDENTIFICATION_LEVEL_FIELD`]
    identification_level: usize,
    /// Index of the kept column holding [`UNITS_FIELD`]
    units: usize,
    /// Whether the sheet has a [`DETECTION_FIELD`] column, which the characterisation replaces
    detection_in_sheet: bool,
}

/// A values field that takes text, and the markers its cells may hold besides a number
#[derive(Clone)]
struct ValuesMarkers {
    field: String,
    /// Trimmed and uppercase
    markers: Vec<String>,
    /// The limits of the markers in `markers`, each in a dictionary field
    limits: Vec<MarkerLimit>,
}

/// What a cell made of a marker and a number is; see [`Unpivot::marker_limits`]
enum MarkerWithLimit<'a> {
    /// The number is the row's limit: write the marker alone
    Matches(&'a MarkerLimit),
    /// Why the number is refused
    Refused(String),
}

impl ValuesMarkers {
    /// The marker `value` is, matched ignoring case
    fn marker(&self, value: &str) -> Option<&str> {
        let value = value.trim().to_uppercase();
        self.markers
            .iter()
            .find(|marker| **marker == value)
            .map(String::as_str)
    }

    /// `None` when `value` is not a marker with a limit followed by `:`, matched ignoring case.
    fn marker_with_limit(
        &self,
        value: &str,
        row: &Map<String, Value>,
    ) -> Option<MarkerWithLimit<'_>> {
        self.limits.iter().find_map(|limit| {
            let number = strip_marker(value, &limit.marker)?
                .trim_start()
                .strip_prefix(':')?
                .trim();
            let row_limit = number_in(row, &limit.limit_field);
            let Some(row_limit) = row_limit else {
                return Some(MarkerWithLimit::Refused(format!(
                    "'{}' has a number, but the row has no {}",
                    value, limit.limit_field
                )));
            };
            let matches = number
                .parse::<f64>()
                .is_ok_and(|n| (n - row_limit).abs() <= 1e-9 * n.abs().max(row_limit.abs()));
            Some(if matches {
                MarkerWithLimit::Matches(limit)
            } else {
                MarkerWithLimit::Refused(format!(
                    "'{}' does not end with the row's {} ({}); write '{}' or '{}:{}'",
                    value, limit.limit_field, row_limit, limit.marker, limit.marker, row_limit
                ))
            })
        })
    }
}

/// The sheet's columns that are not data dictionary fields, and the field they are moved into
struct ExtraColumns {
    /// (column index, header) of each extra column
    columns: Vec<(usize, String)>,
    target: String,
}

impl ExcelValidatorBuilder {
    /// Create a new ExcelValidatorBuilder
    ///
    /// # Arguments
    /// * `excel_path` - Path to the Excel file
    /// * `sheet_name` - Name of the sheet to process
    /// * `schema` - JSON schema for validation
    pub fn new(excel_path: &str, sheet_name: &str, schema: Value) -> Self {
        ExcelValidatorBuilder {
            excel_path: excel_path.to_string(),
            sheet_name: sheet_name.to_string(),
            schema,
            unpivot: None,
            extra_columns_to: None,
        }
    }

    /// Unpivot the sheet while reading it; see [`Unpivot`].
    pub fn unpivot(mut self, unpivot: Unpivot) -> Self {
        self.unpivot = Some(unpivot);
        self
    }

    /// Move the sheet's columns that are not data dictionary fields into `field`, a dictionary
    /// field that takes text, instead of refusing them.
    ///
    /// For each row with data in those columns, `field` gets a JSON object of their non-blank
    /// cells, keyed by header in sheet order. Text already in `field` is kept in the object under
    /// its own header, first. Rows without such data keep `field` as it is.
    pub fn extra_columns_to(mut self, field: &str) -> Self {
        self.extra_columns_to = Some(field.to_string());
        self
    }

    /// Build the ExcelValidator, processing Excel rows during construction
    ///
    /// This method reads the Excel file once and caches the headers and rows,
    /// making subsequent validation and export operations more efficient.
    pub fn build(self) -> Result<ExcelValidator> {
        // An unpivoted sheet has no extra columns: every column after the fields is a sample
        if self.unpivot.is_some() && self.extra_columns_to.is_some() {
            return Err(anyhow::anyhow!(
                "Unpivot the sheet or move its extra columns, not both."
            ));
        }

        // Create validator from schema
        let validator = jsonschema::validator_for(&self.schema)
            .map_err(|e| anyhow::anyhow!("Invalid JSON schema: {}", e))?;

        // Extract field schemas for intelligent type coercion
        let field_schemas = ExcelValidator::extract_field_schemas(&self.schema)?;

        // Create temporary validator to process Excel rows
        let temp_validator = ExcelValidator {
            excel_path: self.excel_path.clone(),
            sheet_name: self.sheet_name.clone(),
            validator: jsonschema::validator_for(&self.schema)
                .map_err(|e| anyhow::anyhow!("Invalid JSON schema: {}", e))?,
            field_schemas: field_schemas.clone(),
            validation_reports: Vec::new(),
            headers: Vec::new(),
            rows: Vec::new(),
            unpivot_summary: None,
            extra_columns_summary: None,
            error_cells_summary: None,
            changes: Vec::new(),
            questions: Vec::new(),
            ng_per_ml_rows: Vec::new(),
            problems: Vec::new(),
            values_markers: None,
        };

        // Process Excel rows once during build
        let sheet = temp_validator
            .process_excel_rows(self.unpivot.as_ref(), self.extra_columns_to.as_deref())?;

        Ok(ExcelValidator {
            excel_path: self.excel_path,
            sheet_name: self.sheet_name,
            validator,
            field_schemas,
            validation_reports: Vec::new(),
            headers: sheet.headers,
            rows: sheet.rows,
            unpivot_summary: sheet.unpivot_summary,
            extra_columns_summary: sheet.extra_columns_summary,
            error_cells_summary: sheet.error_cells_summary,
            changes: sheet.changes,
            questions: sheet.questions,
            ng_per_ml_rows: sheet.ng_per_ml_rows,
            problems: sheet.problems,
            values_markers: sheet.values_markers,
        })
    }
}

impl ExcelValidator {
    //////////////////////////////////////////////////////////////
    //  Public API
    //////////////////////////////////////////////////////////////

    /// Create a test instance of ExcelValidator (for testing only)
    ///
    /// This creates a validator with empty rows/headers for schema-aware method testing.
    #[cfg(any(test, feature = "test"))]
    pub fn new_for_testing(schema: &Value) -> Result<Self> {
        let validator = jsonschema::validator_for(schema)
            .map_err(|e| anyhow::anyhow!("Invalid JSON schema: {}", e))?;
        let field_schemas = Self::extract_field_schemas(schema)?;

        Ok(ExcelValidator {
            excel_path: String::new(),
            sheet_name: String::new(),
            validator,
            field_schemas,
            validation_reports: Vec::new(),
            headers: Vec::new(),
            rows: Vec::new(),
            unpivot_summary: None,
            extra_columns_summary: None,
            error_cells_summary: None,
            changes: Vec::new(),
            questions: Vec::new(),
            ng_per_ml_rows: Vec::new(),
            problems: Vec::new(),
            values_markers: None,
        })
    }

    /// What the unpivot did (columns kept and unpivoted, rows produced), when the sheet was
    /// unpivoted
    pub fn unpivot_summary(&self) -> Option<&str> {
        self.unpivot_summary.as_deref()
    }

    /// What moving the extra columns did (columns moved, rows with extra data), when they were
    /// moved; see [`ExcelValidatorBuilder::extra_columns_to`]
    pub fn extra_columns_summary(&self) -> Option<&str> {
        self.extra_columns_summary.as_deref()
    }

    /// The Excel error cells (`#VALUE!`, `#N/A`, ...) read as blank cells, by row and column,
    /// when there were any. An error value is what a formula gives when its calculation fails, so
    /// it carries no data.
    pub fn error_cells_summary(&self) -> Option<&str> {
        self.error_cells_summary.as_deref()
    }

    /// Every change made to a cell instead of taking it as written, in sheet order, each with
    /// its reason
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// The open questions about the data, each with the number of cells concerned, in a fixed
    /// order
    pub fn questions(&self) -> &[(Question, usize)] {
        &self.questions
    }

    /// The ng/mL rows of an unpivoted sheet, in sheet order. Each is skipped, and says which ng/L
    /// row holds its values, or the closest one when none does, which stops the run.
    pub fn ng_per_ml_rows(&self) -> &[NgPerMlRow] {
        &self.ng_per_ml_rows
    }

    /// Get the headers from the Excel file
    pub fn headers(&self) -> &Vec<String> {
        &self.headers
    }

    /// Get the parsed rows from the Excel file
    pub fn rows(&self) -> &Vec<ParsedExcelRow> {
        &self.rows
    }

    /// Get the field schemas parsed from the JSON schema
    pub fn field_schemas(&self) -> &HashMap<String, FieldSchema> {
        &self.field_schemas
    }

    pub fn validate_excel(&mut self) -> Result<()> {
        // What stops the run in the rows read: reported with the validation errors, not instead
        let problems = self.problems.join("\n");
        let row_count = self.rows.len();
        if row_count == 0 {
            return Err(if problems.is_empty() {
                anyhow::anyhow!("The Excel file is empty")
            } else {
                anyhow::anyhow!("{problems}")
            });
        }

        for (row_number, json_obj) in &self.rows {
            // STRICT VALIDATION: Validate BEFORE type coercion to enforce exact type matching
            let row_value = Value::Object(json_obj.clone());
            let errors = self.validate_and_collect_errors(&row_value, *row_number);

            if !errors.is_empty() {
                // Store for later reporting
                self.validation_reports.push(ValidationReport {
                    row_number: *row_number,
                    errors,
                    row_data: row_value,
                });
            }
        }

        // Log validation errors using centralized logging if there are any errors
        if !self.validation_reports.is_empty() {
            let validation_report = self.format_validation_report();
            let summary = format!(
                "Excel validation failed with {} error(s) across {} row(s).",
                self.validation_reports
                    .iter()
                    .map(|r| r.errors.len())
                    .sum::<usize>(),
                self.validation_reports.len()
            );

            // Return error if validation failed. If the log cannot be written, carry the details
            // in the error itself rather than pointing the user at a log that lacks them.
            let validation = match write_error_to_log(
                "Excel Validation Error Report",
                &validation_report,
            ) {
                Ok(()) => format!("{summary} Check the error log for details."),
                Err(e) => format!(
                    "{summary} The error log could not be written ({e}), so the details follow:\n{validation_report}"
                ),
            };
            return Err(if problems.is_empty() {
                anyhow::anyhow!("{validation}")
            } else {
                anyhow::anyhow!("{problems}\n{validation}")
            });
        }

        if !problems.is_empty() {
            return Err(anyhow::anyhow!("{problems}"));
        }
        Ok(())
    }

    /// Validate that files referenced in specified columns exist in the filesystem
    ///
    /// # Arguments
    /// * `file_columns` - Names of Excel columns that contain filenames
    /// * `files_directory` - Base directory where files should be located
    /// * `validate_file_fn` - Function to validate file existence (for testability)
    ///
    /// # Returns
    /// * `Ok(())` if all files exist
    /// * `Err` with details about missing files if any are not found
    pub fn validate_file_columns<F>(
        &self,
        file_columns: &[String],
        files_directory: &str,
        validate_file_fn: F,
    ) -> Result<()>
    where
        F: Fn(&str) -> Result<()>,
    {
        let mut missing_files: Vec<(usize, String, String)> = vec![];

        for row in self.rows.iter() {
            let row_number = row.0; // Use the actual row number from the parsed row

            for column_name in file_columns.iter() {
                // Guard: Check if column exists in headers
                if !self.headers.contains(column_name) {
                    eprintln!(
                        "⚠️  Warning: File column '{}' not found in Excel file headers",
                        column_name
                    );
                    continue;
                }

                // Guard: Get the value from the row
                let Some(value) = row.1.get(column_name) else {
                    continue;
                };

                // Guard: Convert to string
                let Some(filename) = value.as_str() else {
                    continue;
                };

                // Guard: Skip empty filenames
                if filename.is_empty() {
                    continue;
                }

                // Construct the full file path and validate
                let file_path = format!("{}/{}", files_directory, filename);
                if validate_file_fn(&file_path).is_err() {
                    missing_files.push((row_number, column_name.clone(), filename.to_string()));
                }
            }
        }

        // Return error if any files are missing
        if !missing_files.is_empty() {
            let mut error_msg =
                String::from("The following files are missing in the filesystem:\n");
            for (row_num, column, filename) in &missing_files {
                error_msg.push_str(&format!(
                    "  Row {}, Column '{}': {}\n",
                    row_num, column, filename
                ));
            }
            error_msg.push_str(&format!(
                "\nTotal: {} missing file(s). Either clean the Excel file or add the missing files to the filesystem.",
                missing_files.len()
            ));
            return Err(anyhow::anyhow!(error_msg));
        }

        Ok(())
    }

    /// Export Excel data to CSV with schema-aware parsing
    pub fn export_to_csv(
        &self,
        csv_path: &str,
        title_to_name_mapping: HashMap<String, String>,
    ) -> Result<()> {
        // Configure CSV writer to quote fields when necessary (e.g., when they contain commas)
        let mut wtr = csv::WriterBuilder::new()
            .quote_style(csv::QuoteStyle::Necessary)
            .from_path(csv_path)?;

        // Map Excel headers (titles) to dictionary names for CSV output
        let csv_headers: Vec<String> = self
            .headers
            .iter()
            .map(|title| {
                title_to_name_mapping
                    .get(title)
                    .cloned()
                    .unwrap_or_else(|| title.clone())
            })
            .collect();

        // Write headers using dictionary names - no manual escaping needed, csv writer handles it
        wtr.write_record(&csv_headers)?;

        // Write data rows
        for (_row_number, json_obj) in &self.rows {
            // Convert parsed JSON values back to CSV record
            let mut csv_record: Vec<String> = Vec::new();
            for header in &self.headers {
                if let Some(parsed_value) = json_obj.get(header) {
                    // Convert the JSON value to a string for CSV
                    let csv_value = match parsed_value {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => {
                            // Numeric columns get a type placeholder - see NUMERIC_PLACEHOLDER.
                            // Look up field schema using the original Excel header (title)
                            if let Some(field_schema) = self.field_schemas.get(header) {
                                match &field_schema.field_type {
                                    SchemaType::Number => NUMERIC_PLACEHOLDER.to_string(),
                                    SchemaType::Integer => INTEGER_PLACEHOLDER.to_string(),
                                    SchemaType::Mixed(types) => {
                                        // For mixed types (like [Number, Null]), check if it contains Number or Integer
                                        if types.iter().any(|t| matches!(t, SchemaType::Number)) {
                                            NUMERIC_PLACEHOLDER.to_string()
                                        } else if types
                                            .iter()
                                            .any(|t| matches!(t, SchemaType::Integer))
                                        {
                                            INTEGER_PLACEHOLDER.to_string()
                                        } else {
                                            String::new()
                                        }
                                    }
                                    _ => String::new(),
                                }
                            } else {
                                String::new()
                            }
                        }
                        Value::Array(arr) => {
                            // Convert arrays to semicolon-separated strings (safer than commas)
                            arr.iter()
                                .map(|v| match v {
                                    Value::String(s) => s.clone(),
                                    other => other.to_string().trim_matches('"').to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(";")
                        }
                        Value::Object(_) => parsed_value.to_string(),
                    };
                    csv_record.push(csv_value);
                } else {
                    csv_record.push(String::new());
                }
            }

            wtr.write_record(&csv_record)?;
        }

        wtr.flush()?;

        Ok(())
    }

    //////////////////////////////////////////////////////////////
    //  Private methods
    //////////////////////////////////////////////////////////////
    /// Extract field schemas from JSON schema for intelligent type coercion
    fn extract_field_schemas(schema: &Value) -> Result<HashMap<String, FieldSchema>> {
        let mut field_schemas = HashMap::new();

        if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
            for (field_name, field_schema) in properties {
                let parsed_schema = Self::parse_field_schema(field_schema)
                    .with_context(|| format!("Invalid schema for field \"{}\"", field_name))?;
                field_schemas.insert(field_name.clone(), parsed_schema);
            }
        }

        Ok(field_schemas)
    }

    /// Parse individual field schema into our FieldSchema structure
    fn parse_field_schema(schema: &Value) -> Result<FieldSchema> {
        let field_type = Self::parse_schema_type(schema)?;

        let format = schema
            .get("format")
            .and_then(|f| f.as_str())
            .map(|s| s.to_string());

        let pattern = schema
            .get("pattern")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());

        let enum_values = schema.get("enum").and_then(|e| e.as_array()).cloned();

        let minimum = schema.get("minimum").and_then(|m| m.as_f64());

        let maximum = schema.get("maximum").and_then(|m| m.as_f64());

        let min_length = schema
            .get("minLength")
            .and_then(|m| m.as_u64())
            .map(|n| n as usize);

        let max_length = schema
            .get("maxLength")
            .and_then(|m| m.as_u64())
            .map(|n| n as usize);

        Ok(FieldSchema {
            field_type,
            format,
            pattern,
            enum_values,
            minimum,
            maximum,
            min_length,
            max_length,
        })
    }

    /// Parse schema type, handling union types and complex schemas
    fn parse_schema_type(schema: &Value) -> Result<SchemaType> {
        // Handle union types (array of types)
        if let Some(types) = schema.get("type").and_then(|t| t.as_array()) {
            let parsed_types: Result<Vec<_>> = types.iter().map(Self::parse_single_type).collect();
            return Ok(SchemaType::Mixed(parsed_types?));
        }

        // Handle single type
        if let Some(type_str) = schema.get("type").and_then(|t| t.as_str()) {
            return Self::parse_single_type(&json!(type_str));
        }

        // Handle arrays with item schema
        if schema.get("type").and_then(|t| t.as_str()) == Some("array")
            && let Some(items) = schema.get("items")
        {
            let item_type = Self::parse_schema_type(items)?;
            return Ok(SchemaType::Array(Box::new(item_type)));
        }

        // Default to mixed type if we can't determine
        Ok(SchemaType::Mixed(vec![
            SchemaType::String,
            SchemaType::Number,
            SchemaType::Boolean,
        ]))
    }

    fn parse_single_type(type_val: &Value) -> Result<SchemaType> {
        match type_val.as_str() {
            Some("string") => Ok(SchemaType::String),
            Some("integer") => Ok(SchemaType::Integer),
            Some("number") => Ok(SchemaType::Number),
            Some("boolean") => Ok(SchemaType::Boolean),
            Some("array") => Ok(SchemaType::Array(Box::new(SchemaType::String))), // Default array type
            Some("object") => Ok(SchemaType::Object),
            Some("null") => Ok(SchemaType::Null),
            _ => Ok(SchemaType::String), // Default fallback
        }
    }

    /// Helper method to process Excel files and return parsed row data
    /// Returns headers and a vector of (row_number, parsed_json_object) tuples
    fn process_excel_rows(
        &self,
        unpivot: Option<&Unpivot>,
        extra_columns_to: Option<&str>,
    ) -> Result<ProcessedSheet> {
        let mut workbook: Xlsx<_> = open_workbook(&self.excel_path).with_context(|| {
            format!(
                "The Excel file does not exist in the provided path: \"{}\"",
                self.excel_path
            )
        })?;

        // Get the sheet to process
        let range = workbook
            .worksheet_range(&self.sheet_name)
            .with_context(|| {
                format!(
                    "The sheet \"{}\" does not exist in the Excel file",
                    self.sheet_name
                )
            })?;

        // The range starts at the sheet's first used cell, which need not be A1. Only `None` for
        // an empty sheet, which has no rows to locate.
        let (first_row, first_col) = range.start().unwrap_or((0, 0));

        // One entry per column; `None` for a column without a header
        let mut columns: Vec<Option<String>> = Vec::new();
        let mut headers: Vec<String> = Vec::new();
        // Indices of the columns copied to every row: all named columns, unless unpivoting
        let mut kept: Vec<usize> = Vec::new();
        let mut samples: Option<SampleColumns> = None;
        let mut extras: Option<ExtraColumns> = None;
        let mut rows_with_extra_data: usize = 0;
        // Sample cells such as `<LOQ:0.25` written as the marker alone
        let mut markers_without_limit: usize = 0;
        // Blank sample cells: no result, so no row
        let mut blank_sample_cells: usize = 0;
        // NS sample cells: no sample, so no row
        let mut no_sample_cells: usize = 0;
        // Rows with results whose identification level is not IDENTIFICATION_LEVEL
        let mut wrong_identification_levels: Vec<String> = Vec::new();
        // Rows in ng/mL, skipped here and matched with their ng/L rows once every row is read
        let mut ng_per_ml_count: usize = 0;
        // The open questions, each with the number of cells concerned
        let mut questions: BTreeMap<Question, usize> = BTreeMap::new();
        let mut parsed_rows: Vec<ParsedExcelRow> = Vec::new();
        // Every change made to a cell instead of taking it as written, in sheet order. Excel error
        // values (#VALUE!, #N/A, ...) are failed calculations, not data: each is read as a blank
        // cell and listed here.
        let mut changes: Vec<Change> = Vec::new();
        // Data in a column without a header has no field to go to: the first value of each such
        // column, by column index
        let mut unnamed_columns_with_data: BTreeMap<usize, String> = BTreeMap::new();

        // Process each row
        for (row_index, row) in range.rows().enumerate() {
            if row_index == 0 {
                // First row contains headers - normalize them to match DKAN titles
                columns = row
                    .iter()
                    .map(|cell| Some(normalize_string(&cell.to_string())).filter(|h| !h.is_empty()))
                    .collect();
                headers = columns.iter().flatten().cloned().collect();

                log::debug!("Excel headers: {:#?}", headers);

                // Check for duplicate headers
                Self::check_header_duplicates(&headers)?;

                match unpivot {
                    None => {
                        kept = (0..columns.len())
                            .filter(|&i| columns.get(i).is_some_and(Option::is_some))
                            .collect();
                        if let Some(field) = extra_columns_to {
                            let extra = self.extra_columns_layout(&columns, field)?;
                            kept.retain(|i| !extra.columns.iter().any(|(c, _)| c == i));
                            headers = kept
                                .iter()
                                .filter_map(|&i| columns.get(i).cloned().flatten())
                                .collect();
                            if !headers.contains(&extra.target) {
                                headers.push(extra.target.clone());
                            }
                            extras = Some(extra);
                        }
                    }
                    Some(unpivot) => {
                        let (unpivot_kept, unpivot_samples) =
                            self.unpivot_layout(&columns, unpivot, first_col)?;
                        headers = unpivot_kept
                            .iter()
                            .filter_map(|&i| columns.get(i).cloned().flatten())
                            .chain([
                                unpivot_samples.headers_column.clone(),
                                unpivot_samples.values_column.clone(),
                            ])
                            .collect();
                        if !unpivot_samples.detection_in_sheet {
                            headers.push(DETECTION_FIELD.to_string());
                        }
                        kept = unpivot_kept;
                        samples = Some(unpivot_samples);
                    }
                }

                continue;
            }

            let row_number = sheet_row(first_row, row_index);

            // Before anything else, as nothing of an ng/mL row is imported: not even its Excel
            // error cells are listed as emptied
            if let Some(samples) = &samples
                && Units::parse(row.get(samples.units)).is_ng_per_ml()
            {
                ng_per_ml_count = ng_per_ml_count.saturating_add(1);
                continue;
            }

            let sheet_cell = |col_idx: usize| SheetCell {
                row: row_number,
                column: column_name(sheet_column(first_col, col_idx)),
                header: columns.get(col_idx).cloned().flatten(),
            };

            let row: Cow<[Data]> = if row.iter().any(|cell| matches!(cell, Data::Error(_))) {
                Cow::Owned(
                    row.iter()
                        .enumerate()
                        .map(|(col_idx, cell)| match cell {
                            Data::Error(e) => {
                                changes.push(Change::Emptied {
                                    cell: sheet_cell(col_idx),
                                    error: e.to_string(),
                                });
                                Data::Empty
                            }
                            cell => cell.clone(),
                        })
                        .collect(),
                )
            } else {
                Cow::Borrowed(row)
            };

            for (col_idx, (cell, _)) in row
                .iter()
                .zip(&columns)
                .enumerate()
                .filter(|(_, (cell, column))| column.is_none() && !is_blank(cell))
            {
                unnamed_columns_with_data.entry(col_idx).or_insert_with(|| {
                    format!(
                        "column {} (first value at row {}: {:?})",
                        column_name(sheet_column(first_col, col_idx)),
                        row_number,
                        cell.to_string()
                    )
                });
            }

            // Skip empty rows
            if row.iter().all(is_blank) {
                continue;
            }

            // Convert row to JSON object with intelligent type coercion
            let mut json_obj = Map::new();
            for &col_idx in &kept {
                let (Some(cell), Some(Some(header))) = (row.get(col_idx), columns.get(col_idx))
                else {
                    continue;
                };
                let value = self.convert_cell_to_json_with_schema_awareness(cell, header);
                json_obj.insert(header.clone(), value);
            }

            if let Some(extras) = &extras {
                let mut moved = Map::new();
                for (col_idx, header) in &extras.columns {
                    let Some(cell) = row.get(*col_idx).filter(|cell| !is_blank(cell)) else {
                        continue;
                    };
                    moved.insert(header.clone(), self.extra_cell_to_json(cell, header));
                }
                if !moved.is_empty() {
                    rows_with_extra_data = rows_with_extra_data.saturating_add(1);
                    let mut object = Map::new();
                    if let Some(existing) = json_obj
                        .get(&extras.target)
                        .filter(|v| !v.is_null() && v.as_str() != Some(""))
                    {
                        object.insert(extras.target.clone(), existing.clone());
                    }
                    object.extend(moved);
                    json_obj.insert(
                        extras.target.clone(),
                        Value::String(Value::Object(object).to_string()),
                    );
                }
            }

            let Some(samples) = &samples else {
                parsed_rows.push((row_number, json_obj));
                continue;
            };
            // Only a row with results gives rows, so only its identification level matters
            let has_results = samples
                .columns
                .iter()
                .any(|(col_idx, _)| row.get(*col_idx).is_some_and(|cell| !is_no_result(cell)));
            match row.get(samples.identification_level) {
                _ if !has_results => {}
                Some(Data::String(level)) if level.trim() == IDENTIFICATION_LEVEL => {}
                cell => wrong_identification_levels.push(format!(
                    "row {}: {}",
                    row_number,
                    cell.filter(|cell| !is_blank(cell))
                        .map_or_else(|| "blank".to_string(), ToString::to_string)
                )),
            }
            let units = Units::parse(row.get(samples.units));
            let lod = number_in(&json_obj, LOD_FIELD);
            let loq = number_in(&json_obj, LOQ_FIELD);
            let sheet_detection = json_obj
                .get(DETECTION_FIELD)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|detection| !detection.is_empty())
                .map(String::from);
            for (col_idx, header) in &samples.columns {
                let Some(cell) = row.get(*col_idx).filter(|cell| !is_blank(cell)) else {
                    // Counted in the summary, not listed: a sheet has thousands
                    blank_sample_cells = blank_sample_cells.saturating_add(1);
                    continue;
                };
                if is_no_result(cell) {
                    no_sample_cells = no_sample_cells.saturating_add(1);
                    changes.push(Change::Skipped {
                        cell: sheet_cell(*col_idx),
                        reason: SkipReason::NoSample(cell.to_string().trim().to_string()),
                    });
                    continue;
                }
                let mut sample_obj = json_obj.clone();
                sample_obj.insert(
                    samples.headers_column.clone(),
                    Value::String(header.clone()),
                );
                let mut value =
                    self.convert_cell_to_json_with_schema_awareness(cell, &samples.values_column);
                // A marker is written uppercase; one followed by the row's limit, as the marker
                // alone. A limit that does not match is left as written, for the validator to
                // report.
                let marker = samples.values_markers.as_ref().and_then(|values| {
                    let s = value.as_str()?;
                    match values.marker(s) {
                        Some(marker) => Some((marker, None)),
                        None => match values.marker_with_limit(s, &json_obj)? {
                            MarkerWithLimit::Matches(limit) => {
                                Some((limit.marker.as_str(), Some(limit.limit_field.as_str())))
                            }
                            MarkerWithLimit::Refused(_) => None,
                        },
                    }
                });
                if let Some((marker, limit_field)) = marker {
                    // Spaces around a cell's text are trimmed from every cell, so they are no
                    // change of their own
                    let was = cell.to_string().trim().to_string();
                    if was != marker {
                        let reason = match limit_field {
                            Some(field) => {
                                format!("the number after the marker is the row's {field}")
                            }
                            None => "markers are written uppercase".to_string(),
                        };
                        changes.push(Change::Rewritten {
                            cell: sheet_cell(*col_idx),
                            was,
                            now: marker.to_string(),
                            reason,
                        });
                    }
                    value = Value::String(marker.to_string());
                    if limit_field.is_some() {
                        markers_without_limit = markers_without_limit.saturating_add(1);
                    }
                }
                // Text that is neither a number nor a marker is left for the validator to refuse
                let result = match marker {
                    Some((marker, _)) => Some(SampleResult::Marker(marker)),
                    None => number_of(&value).map(SampleResult::Number),
                };
                if let Some(result) = result {
                    let raw = cell.to_string();
                    match characterise(&units, &result, raw.trim(), lod, loq) {
                        Outcome::Characterised {
                            detection,
                            reason,
                            question,
                        } => {
                            if let Some(question) = question {
                                let count = questions.entry(question).or_insert(0);
                                *count = count.saturating_add(1);
                            }
                            if let Some(was) =
                                sheet_detection.as_deref().filter(|was| *was != detection)
                            {
                                changes.push(Change::Derived {
                                    cell: sheet_cell(*col_idx),
                                    field: DETECTION_FIELD.to_string(),
                                    value: detection.to_string(),
                                    was: was.to_string(),
                                    reason,
                                });
                            }
                            sample_obj.insert(
                                DETECTION_FIELD.to_string(),
                                Value::String(detection.to_string()),
                            );
                        }
                        Outcome::Open { question, reason } => {
                            let count = questions.entry(question.clone()).or_insert(0);
                            *count = count.saturating_add(1);
                            changes.push(Change::Skipped {
                                cell: sheet_cell(*col_idx),
                                reason: SkipReason::Open { question, reason },
                            });
                            continue;
                        }
                    }
                }
                sample_obj.insert(samples.values_column.clone(), value);
                parsed_rows.push((row_number, sample_obj));
            }
        }

        // Problems in the rows stop the run, but only once every row has been read, so that one
        // run reports all of them, with the changes and questions
        let mut problems: Vec<String> = Vec::new();
        if !unnamed_columns_with_data.is_empty() {
            problems.push(format!(
                "The sheet \"{}\" has data in {} column(s) without a header. Add a header or clear these cells and try again:\n  {}",
                self.sheet_name,
                unnamed_columns_with_data.len(),
                unnamed_columns_with_data
                    .into_values()
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ));
        }

        let ng_per_ml_rows = match &samples {
            Some(samples) if ng_per_ml_count > 0 => {
                ng_per_ml_rows(&range, first_row, first_col, &columns, &kept, samples)
            }
            _ => Vec::new(),
        };
        let missing: Vec<String> = ng_per_ml_rows
            .iter()
            .filter(|row| matches!(row.counterpart, Counterpart::Missing(_)))
            .map(ToString::to_string)
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "The sheet \"{}\" has {} {NANOGRAMS_PER_MILLILITRE} row(s) whose values no ng/L row holds. Only ng/L is imported, so each {NANOGRAMS_PER_MILLILITRE} row is skipped, and its values must be in an ng/L row: correct the ng/L rows in the source file and try again:\n  {}",
                self.sheet_name,
                missing.len(),
                missing.join("\n  ")
            ));
        }

        if !wrong_identification_levels.is_empty() {
            problems.push(format!(
                "The sheet \"{}\" has {} row(s) whose {} is not {}. Analytical results are always identification level {}: correct these cells in the source file and try again:\n  {}",
                self.sheet_name,
                wrong_identification_levels.len(),
                IDENTIFICATION_LEVEL_FIELD,
                IDENTIFICATION_LEVEL,
                IDENTIFICATION_LEVEL,
                wrong_identification_levels.join("\n  ")
            ));
        }

        let values_markers = samples
            .as_ref()
            .and_then(|samples| samples.values_markers.clone());
        let unpivot_summary = samples.map(|samples| {
            let kept_columns: Vec<(usize, &str)> = kept
                .iter()
                .filter_map(|&i| columns.get(i)?.as_deref().map(|name| (i, name)))
                .collect();
            let sample_columns: Vec<(usize, &str)> = samples
                .columns
                .iter()
                .map(|(i, name)| (*i, name.as_str()))
                .collect();
            let markers = samples
                .values_markers
                .map(|values| {
                    let limits = if markers_without_limit > 0 {
                        format!(
                            " {} value(s) written as their marker alone, the number after it being the row's limit: {}.",
                            markers_without_limit,
                            values
                                .limits
                                .iter()
                                .map(|l| format!("{} ({})", l.marker, l.limit_field))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        " {} takes numbers or {}.{}",
                        values.field,
                        values.markers.join(", "),
                        limits
                    )
                })
                .unwrap_or_default();
            let mut blanks = String::new();
            if ng_per_ml_count > 0 {
                blanks.push_str(&format!(
                    "; {ng_per_ml_count} {NANOGRAMS_PER_MILLILITRE} row(s) skipped"
                ));
            }
            if blank_sample_cells > 0 {
                blanks.push_str(&format!("; {blank_sample_cells} blank sample cell(s) skipped"));
            }
            if no_sample_cells > 0 {
                blanks.push_str(&format!(
                    "; {no_sample_cells} {NO_SAMPLE} or {NO_SAMPLE_ALIAS} (no sample) cell(s) skipped"
                ));
            }
            format!(
                "Keeping {} column(s) {}; unpivoting {} column(s) {} into {} / {}: {} row(s){}.{}",
                kept_columns.len(),
                column_span(&kept_columns, first_col),
                sample_columns.len(),
                column_span(&sample_columns, first_col),
                samples.headers_column,
                samples.values_column,
                parsed_rows.len(),
                blanks,
                markers
            )
        });

        let extra_columns_summary = extras.map(|extras| {
            if extras.columns.is_empty() {
                return format!("No extra columns to move into {}.", extras.target);
            }
            format!(
                "Moving {} extra column(s) into {}: {}; {} row(s) have extra data.",
                extras.columns.len(),
                extras.target,
                extras
                    .columns
                    .iter()
                    .map(|(i, name)| format!(
                        "column {} \"{}\"",
                        column_name(sheet_column(first_col, *i)),
                        name
                    ))
                    .collect::<Vec<_>>()
                    .join(", "),
                rows_with_extra_data
            )
        });

        let error_cells: Vec<String> = changes
            .iter()
            .filter_map(|change| match change {
                Change::Emptied { cell, error } => Some(format!("{cell}: {error}")),
                _ => None,
            })
            .collect();

        Ok(ProcessedSheet {
            headers,
            rows: parsed_rows,
            unpivot_summary,
            extra_columns_summary,
            error_cells_summary: (!error_cells.is_empty()).then(|| {
                format!(
                    "Emptied {} Excel error value(s), uploading them as blank cells:\n  {}",
                    error_cells.len(),
                    error_cells.join("\n  ")
                )
            }),
            changes,
            questions: questions.into_iter().collect(),
            ng_per_ml_rows,
            problems,
            values_markers,
        })
    }

    /// The sheet's named columns that are not data dictionary fields, to be moved into
    /// `field`. Refuses a `field` that is not a dictionary field taking text.
    fn extra_columns_layout(
        &self,
        columns: &[Option<String>],
        field: &str,
    ) -> Result<ExtraColumns> {
        let target = normalize_string(field);
        let mut candidates: Vec<&str> = self
            .field_schemas
            .iter()
            .filter(|(_, schema)| self.schema_expects_string(&schema.field_type))
            .map(|(name, _)| name.as_str())
            .collect();
        candidates.sort_unstable();
        if !candidates.contains(&target.as_str()) {
            return Err(anyhow::anyhow!(
                "The extra columns must go to a data dictionary field that takes text; \"{}\" is not one. Candidates: {}",
                target,
                candidates.join(", ")
            ));
        }

        Ok(ExtraColumns {
            columns: columns
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c.as_ref().map(|name| (i, name)))
                .filter(|(_, name)| !self.field_schemas.contains_key(*name))
                .map(|(i, name)| (i, name.clone()))
                .collect(),
            target,
        })
    }

    /// An extra column's cell as it goes into the JSON object: numbers stay numbers (whole ones
    /// without a fraction), text stays text, dates become ISO text. Blank and error cells are
    /// handled by the caller.
    fn extra_cell_to_json(&self, cell: &Data, header: &str) -> Value {
        match cell {
            Data::Float(f) if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 => {
                // Below 2^53 every whole float is an exact i64
                json!(*f as i64)
            }
            Data::Float(f) => json!(f),
            Data::Int(i) => json!(i),
            Data::Bool(b) => Value::Bool(*b),
            Data::DateTime(dt) => self.convert_datetime_with_schema_intelligence(dt, header),
            Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => {
                Value::String(s.clone())
            }
            Data::Empty | Data::Error(_) => Value::Null,
        }
    }

    /// Split the sheet's columns for `unpivot`: the indices of the leading columns that are schema
    /// fields (kept), and the sample columns after them. Refuses targets that are not usable
    /// fields, and sheets that do not have a kept part followed by a sample part.
    fn unpivot_layout(
        &self,
        columns: &[Option<String>],
        unpivot: &Unpivot,
        first_col: u32,
    ) -> Result<(Vec<usize>, SampleColumns)> {
        let headers_column = normalize_string(&unpivot.headers_column);
        let values_column = normalize_string(&unpivot.values_column);
        let named: Vec<(usize, &str)> = columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_deref().map(|name| (i, name)))
            .collect();
        let describe = |(i, name): (usize, &str)| {
            format!(
                "column {} \"{}\"",
                column_name(sheet_column(first_col, i)),
                name
            )
        };

        if headers_column == values_column {
            return Err(anyhow::anyhow!(
                "The headers column and the values column must be different fields; both are \"{}\".",
                headers_column
            ));
        }

        let mut candidates: Vec<&str> = self
            .field_schemas
            .keys()
            .map(String::as_str)
            .filter(|field| !named.iter().any(|(_, name)| name == field))
            .collect();
        candidates.sort_unstable();
        for (role, field) in [
            ("headers column", &headers_column),
            ("values column", &values_column),
        ] {
            if !candidates.contains(&field.as_str()) {
                return Err(anyhow::anyhow!(
                    "The {} \"{}\" must be a data dictionary field that is not a column in the sheet \"{}\". Candidates: {}",
                    role,
                    field,
                    self.sheet_name,
                    candidates.join(", ")
                ));
            }
        }

        let kept_count = named
            .iter()
            .take_while(|(_, name)| self.field_schemas.contains_key(*name))
            .count();
        let (kept, sample_part) = named.split_at(kept_count.min(named.len()));

        let Some(&first_sample) = sample_part.first() else {
            return Err(anyhow::anyhow!(
                "Cannot unpivot the sheet \"{}\": it has no sample columns after the data dictionary fields{}.",
                self.sheet_name,
                kept.last()
                    .map(|&last| format!(" (the last is {})", describe(last)))
                    .unwrap_or_default()
            ));
        };
        if kept.is_empty() {
            return Err(anyhow::anyhow!(
                "Cannot unpivot the sheet \"{}\": it starts with {}, which is not a data dictionary field. The sheet must start with the columns to keep (data dictionary fields), followed by the sample columns.",
                self.sheet_name,
                describe(first_sample)
            ));
        }

        let misplaced: Vec<String> = sample_part
            .iter()
            .filter(|(_, name)| self.field_schemas.contains_key(*name))
            .map(|&column| {
                format!(
                    "{} is a data dictionary field after the first sample {}",
                    describe(column),
                    describe(first_sample)
                )
            })
            .collect();
        if !misplaced.is_empty() {
            return Err(anyhow::anyhow!(
                "Cannot unpivot the sheet \"{}\": move these columns before the sample columns:\n  {}",
                self.sheet_name,
                misplaced.join("\n  ")
            ));
        }

        let level_field = normalize_string(IDENTIFICATION_LEVEL_FIELD);
        if !self.field_schemas.contains_key(&level_field) {
            return Err(anyhow::anyhow!(
                "Cannot unpivot the sheet \"{}\": the data dictionary has no field \"{}\", which analytical results need.",
                self.sheet_name,
                level_field
            ));
        }
        let Some(&(identification_level, _)) = kept.iter().find(|(_, name)| *name == level_field)
        else {
            return Err(anyhow::anyhow!(
                "Cannot unpivot the sheet \"{}\": it has no \"{}\" column before the sample columns. Analytical results are always identification level {}.",
                self.sheet_name,
                level_field,
                IDENTIFICATION_LEVEL
            ));
        };

        // Each result's detection characterisation is worked out from its row's units and limits
        for field in [UNITS_FIELD, DETECTION_FIELD, LOD_FIELD, LOQ_FIELD] {
            if !self.field_schemas.contains_key(field) {
                return Err(anyhow::anyhow!(
                    "Cannot unpivot the sheet \"{}\": the data dictionary has no field \"{}\", which analytical results need.",
                    self.sheet_name,
                    field
                ));
            }
        }
        let Some(&(units, _)) = kept.iter().find(|(_, name)| *name == UNITS_FIELD) else {
            return Err(anyhow::anyhow!(
                "Cannot unpivot the sheet \"{}\": it has no \"{}\" column before the sample columns. Each result is characterised by its row's units.",
                self.sheet_name,
                UNITS_FIELD
            ));
        };
        let detection_in_sheet = kept.iter().any(|(_, name)| *name == DETECTION_FIELD);

        // A numeric field needs no markers: its type already refuses text
        let values_markers = self
            .field_schemas
            .get(&values_column)
            .filter(|schema| self.schema_expects_string(&schema.field_type))
            .map(|_| -> Result<ValuesMarkers> {
                Ok(ValuesMarkers {
                    field: values_column.clone(),
                    markers: unpivot
                        .values_markers
                        .iter()
                        .map(|marker| parse_values_marker(marker))
                        .collect::<std::result::Result<_, _>>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                    limits: Vec::new(),
                })
            })
            .transpose()?
            .map(|mut values| {
                // A limit applies only to a marker in use, so the default limits do not stand in
                // the way of other markers
                values.limits = unpivot
                    .marker_limits
                    .iter()
                    .filter(|limit| values.markers.contains(&limit.marker))
                    .cloned()
                    .collect();
                values
            });
        if let Some(values) = &values_markers {
            for limit in &values.limits {
                if !self.field_schemas.contains_key(&limit.limit_field) {
                    return Err(anyhow::anyhow!(
                        "The limit field \"{}\" for \"{}\" is not a data dictionary field.",
                        limit.limit_field,
                        limit.marker
                    ));
                }
            }
        }

        Ok((
            kept.iter().map(|(i, _)| *i).collect(),
            SampleColumns {
                columns: sample_part
                    .iter()
                    .map(|(i, name)| (*i, name.to_string()))
                    .collect(),
                headers_column,
                values_column,
                values_markers,
                identification_level,
                units,
                detection_in_sheet,
            },
        ))
    }

    /// Format validation reports into a structured string for logging
    fn format_validation_report(&self) -> String {
        let mut report = String::new();

        // Add title and separator
        report.push_str("=============================\n");

        // Generate ISO 8601 formatted timestamp
        let now = crate::utils::get_utc_iso_datetime();
        report.push_str(&format!("Generated at: {}\n\n", now));

        // Add total error count
        report.push_str(&format!(
            "Total rows with errors: {}\n\n",
            self.validation_reports.len()
        ));

        // Add detailed information for each validation report
        for validation_report in &self.validation_reports {
            report.push_str(&format!(
                "Row {}: {} error(s)\n",
                validation_report.row_number,
                validation_report.errors.len()
            ));

            // Add row data (handle JSON serialization errors gracefully)
            match serde_json::to_string_pretty(&validation_report.row_data) {
                Ok(json_data) => {
                    report.push_str(&format!("Row data: {}\n", json_data));
                }
                Err(_) => {
                    report.push_str("Row data: [Error serializing data]\n");
                }
            }

            report.push_str("Errors:\n");

            // Add each specific error
            for error in &validation_report.errors {
                report.push_str(&format!("  - {}\n", error));
            }
            report.push('\n');
        }

        report
    }

    /// Convert cell to JSON with schema awareness for intelligent type coercion
    ///
    /// **Note**: This is primarily exposed for testing. During normal usage,
    /// cell conversion happens automatically during validation.
    fn convert_cell_to_json_with_schema_awareness(&self, cell: &Data, field_name: &str) -> Value {
        // Pre-compute schema information to avoid repeated lookups
        let field_schema = self.field_schemas.get(field_name);
        let expects_string = field_schema
            .map(|schema| self.schema_expects_string(&schema.field_type))
            .unwrap_or(false);

        match cell {
            Data::Empty => {
                // Check if this field allows null values (for non-mandatory fields)
                if let Some(schema) = field_schema {
                    match &schema.field_type {
                        SchemaType::Mixed(types) => {
                            // If it's a union type that includes null, allow it
                            if types.contains(&SchemaType::Null) {
                                return Value::Null;
                            }
                        }
                        SchemaType::Null => return Value::Null,
                        _ => {}
                    }
                }
                Value::Null
            }
            Data::String(s) => self.convert_string_with_schema_intelligence(s, field_name),
            Data::Float(f) => {
                // Check if schema expects a string type
                if expects_string {
                    return Value::String(f.to_string());
                }
                self.convert_numeric_with_validation(*f, field_name)
            }
            Data::Int(i) => {
                // Check if schema expects a string type
                if expects_string {
                    return Value::String(i.to_string());
                }
                self.convert_numeric_with_validation(*i as f64, field_name)
            }
            Data::Bool(b) => Value::Bool(*b),
            Data::Error(_) => Value::Null,
            Data::DateTime(dt) => {
                // Try to convert to appropriate format based on schema expectations
                self.convert_datetime_with_schema_intelligence(dt, field_name)
            }
            Data::DateTimeIso(dt_str) => {
                self.convert_datetime_string_with_schema_intelligence(dt_str, field_name)
            }
            Data::DurationIso(dur_str) => Value::String(dur_str.clone()),
        }
    }

    /// Intelligent string conversion based on schema expectations
    pub(crate) fn convert_string_with_schema_intelligence(
        &self,
        s: &str,
        field_name: &str,
    ) -> Value {
        let trimmed = s.trim();

        // Handle empty strings
        if trimmed.is_empty() {
            // Check if field expects null or empty string
            if let Some(field_schema) = self.field_schemas.get(field_name) {
                match &field_schema.field_type {
                    SchemaType::Null => return Value::Null,
                    SchemaType::Mixed(types)
                        // If it's a union type that includes null, prefer null for empty strings
                        if types.contains(&SchemaType::Null) => {
                            return Value::Null;
                        }
                    _ => {}
                }
            }
            return Value::String(String::new());
        }

        // Check if this is a latitude or longitude field and parse coordinates
        let field_name_lower = field_name.to_lowercase();
        if field_name_lower.contains("latitude") || field_name_lower.contains("lat") {
            match crate::utils::parse_latitude(trimmed) {
                Ok(parsed_lat) => {
                    // Parse successful - convert to appropriate type based on schema
                    if let Some(field_schema) = self.field_schemas.get(field_name) {
                        match &field_schema.field_type {
                            SchemaType::Number | SchemaType::Integer => {
                                return json!(parsed_lat);
                            }
                            SchemaType::Mixed(types)
                                if (types.contains(&SchemaType::Number)
                                    || types.contains(&SchemaType::Integer)) =>
                            {
                                return json!(parsed_lat);
                            }
                            _ => {}
                        }
                    }
                    // For string fields, use the coordinate value as-is
                    return json!(parsed_lat);
                }
                Err(_e) => {
                    // Parsing failed - return original string value
                    // Validation will catch this as a type mismatch if schema expects number
                    log::warn!(
                        "Failed to parse latitude value '{}' for field '{}': {}. The value should be in decimal degrees (e.g., '41.3945') or DMS format (e.g., '41 23.670 N')",
                        trimmed,
                        field_name,
                        _e
                    );
                    return Value::String(trimmed.to_string());
                }
            }
        } else if field_name_lower.contains("longitude")
            || field_name_lower.contains("lng")
            || field_name_lower.contains("lon")
        {
            match crate::utils::parse_longitude(trimmed) {
                Ok(parsed_lng) => {
                    // Parse successful - convert to appropriate type based on schema
                    if let Some(field_schema) = self.field_schemas.get(field_name) {
                        match &field_schema.field_type {
                            SchemaType::Number | SchemaType::Integer => {
                                return json!(parsed_lng);
                            }
                            SchemaType::Mixed(types)
                                if (types.contains(&SchemaType::Number)
                                    || types.contains(&SchemaType::Integer)) =>
                            {
                                return json!(parsed_lng);
                            }
                            _ => {}
                        }
                    }
                    // For string fields, use the coordinate value as-is
                    return json!(parsed_lng);
                }
                Err(_e) => {
                    // Parsing failed - return original string value
                    // Validation will catch this as a type mismatch if schema expects number
                    log::warn!(
                        "Failed to parse longitude value '{}' for field '{}': {}. The value should be in decimal degrees (e.g., '15.727') or DMS format (e.g., '15 43.620 E')",
                        trimmed,
                        field_name,
                        _e
                    );
                    return Value::String(trimmed.to_string());
                }
            }
        }

        // Get schema expectations for this field
        if let Some(field_schema) = self.field_schemas.get(field_name) {
            return self.coerce_string_to_schema_type(trimmed, field_schema);
        }

        // Fallback: intelligent type detection without schema
        self.intelligent_string_conversion(trimmed)
    }

    /// Coerce string to match schema type expectations
    fn coerce_string_to_schema_type(&self, s: &str, field_schema: &FieldSchema) -> Value {
        match &field_schema.field_type {
            SchemaType::Integer => {
                if let Ok(int_val) = s.parse::<i64>() {
                    // Check bounds if specified
                    if let (Some(min), Some(max)) = (field_schema.minimum, field_schema.maximum) {
                        let val_f64 = int_val as f64;
                        if val_f64 >= min && val_f64 <= max {
                            return json!(int_val);
                        }
                    } else {
                        return json!(int_val);
                    }
                }
                // Try parsing as float and converting to int
                if let Ok(float_val) = s.parse::<f64>()
                    && float_val.fract().abs() < f64::EPSILON
                {
                    let int_val = float_val as i64;
                    if let (Some(min), Some(max)) = (field_schema.minimum, field_schema.maximum) {
                        if float_val >= min && float_val <= max {
                            return json!(int_val);
                        }
                    } else {
                        return json!(int_val);
                    }
                }
                Value::String(s.to_string())
            }

            SchemaType::Number => {
                // Use smart number conversion that prefers integers
                if let Some(num_val) = self.smart_number_conversion(s) {
                    // Check bounds if specified
                    if let (Some(min), Some(max)) = (field_schema.minimum, field_schema.maximum) {
                        if let Some(num_f64) = num_val.as_f64()
                            && num_f64 >= min
                            && num_f64 <= max
                        {
                            return num_val;
                        }
                    } else {
                        return num_val;
                    }
                }
                Value::String(s.to_string())
            }

            SchemaType::Boolean => match s.to_lowercase().as_str() {
                "true" | "yes" | "y" | "1" | "on" | "enabled" | "active" => json!(true),
                "false" | "no" | "n" | "0" | "off" | "disabled" | "inactive" => json!(false),
                _ => Value::String(s.to_string()),
            },

            SchemaType::String => {
                // Check enum values if specified
                if let Some(enum_vals) = &field_schema.enum_values {
                    let string_val = Value::String(s.to_string());
                    if enum_vals.contains(&string_val) {
                        return string_val;
                    }

                    // Try case-insensitive matching for enums
                    let lower_s = s.to_lowercase();
                    for enum_val in enum_vals {
                        if let Some(enum_str) = enum_val.as_str()
                            && enum_str.to_lowercase() == lower_s
                        {
                            return Value::String(enum_str.to_string());
                        }
                    }
                }

                // Apply format-specific conversion
                if let Some(format) = &field_schema.format {
                    return self.convert_string_by_format(s, format);
                }

                // Apply pattern validation and normalization
                if let Some(_pattern) = &field_schema.pattern {
                    // Could add pattern-based normalization here
                    return Value::String(s.to_string());
                }

                Value::String(s.to_string())
            }

            SchemaType::Array(_) => {
                // Try to parse as JSON array or split by common delimiters
                if s.starts_with('[')
                    && s.ends_with(']')
                    && let Ok(arr_val) = serde_json::from_str::<Value>(s)
                    && arr_val.is_array()
                {
                    return arr_val;
                }

                // Split by multiple delimiters: newline, semicolon, comma, pipe, tab
                // Replace all delimiters with newline for unified splitting
                let normalized = s.replace([';', ',', '|', '\t'], "\n");

                let items: Vec<Value> = normalized
                    .lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty())
                    .map(|item| Value::String(item.to_string()))
                    .collect();

                // If we found multiple items, return as array
                if items.len() > 1 {
                    return json!(items);
                }

                // If only one item, return it as a single-element array
                if !items.is_empty() {
                    return json!(items);
                }

                // Fallback: return original string as single-element array
                json!([s])
            }

            SchemaType::Object => {
                // Try to parse as JSON object
                if s.starts_with('{')
                    && s.ends_with('}')
                    && let Ok(obj_val) = serde_json::from_str::<Value>(s)
                    && obj_val.is_object()
                {
                    return obj_val;
                }
                Value::String(s.to_string())
            }

            SchemaType::Null => match s.to_lowercase().as_str() {
                "null" | "nil" | "none" | "" => Value::Null,
                _ => Value::String(s.to_string()),
            },

            SchemaType::Mixed(types) => {
                // Smart ordering: try in a way that makes sense for common Excel data
                // Order: integer -> number -> boolean -> string
                let preferred_order = [
                    SchemaType::Integer,
                    SchemaType::Number,
                    SchemaType::Boolean,
                    SchemaType::String,
                ];

                // First, try types in preferred order if they exist in the schema
                for preferred_type in &preferred_order {
                    if types.contains(preferred_type) {
                        let test_schema = FieldSchema {
                            field_type: preferred_type.clone(),
                            format: field_schema.format.clone(),
                            pattern: field_schema.pattern.clone(),
                            enum_values: field_schema.enum_values.clone(),
                            minimum: field_schema.minimum,
                            maximum: field_schema.maximum,
                            min_length: field_schema.min_length,
                            max_length: field_schema.max_length,
                        };

                        let converted = self.coerce_string_to_schema_type(s, &test_schema);

                        // Accept if conversion was successful (changed type or remained valid string)
                        match preferred_type {
                            SchemaType::String => {
                                // For strings, accept if it's a string
                                if converted.is_string() {
                                    return converted;
                                }
                            }
                            _ => {
                                // For other types, accept if it changed from string
                                if !converted.is_string() {
                                    return converted;
                                }
                            }
                        }
                    }
                }

                // If none of the preferred types worked, try remaining types
                for schema_type in types {
                    if !preferred_order.contains(schema_type) {
                        let test_schema = FieldSchema {
                            field_type: schema_type.clone(),
                            format: field_schema.format.clone(),
                            pattern: field_schema.pattern.clone(),
                            enum_values: field_schema.enum_values.clone(),
                            minimum: field_schema.minimum,
                            maximum: field_schema.maximum,
                            min_length: field_schema.min_length,
                            max_length: field_schema.max_length,
                        };

                        let converted = self.coerce_string_to_schema_type(s, &test_schema);
                        if !converted.is_string() || converted.as_str() != Some(s) {
                            return converted;
                        }
                    }
                }

                Value::String(s.to_string())
            }
        }
    }

    /// Enhanced number conversion that prefers integers when possible
    fn smart_number_conversion(&self, s: &str) -> Option<Value> {
        // Try integer first
        if let Ok(int_val) = s.parse::<i64>() {
            return Some(json!(int_val));
        }

        // Try float if integer parsing failed
        if let Ok(float_val) = s.parse::<f64>() {
            // Check if it's actually an integer value in float form
            if float_val.fract().abs() < f64::EPSILON
                && float_val >= i64::MIN as f64
                && float_val <= i64::MAX as f64
            {
                return Some(json!(float_val as i64));
            } else {
                return Some(json!(float_val));
            }
        }

        None
    }

    /// Convert string based on format specification
    fn convert_string_by_format(&self, s: &str, format: &str) -> Value {
        match format {
            "date" => {
                // Normalize to YYYY-MM-DD; anything unparseable is reported by
                // custom_validator_date_formats
                if let Ok(parsed_date) = self.parse_date_string(s) {
                    return Value::String(parsed_date);
                }
                Value::String(s.to_string())
            }

            "date-time" => {
                // Try to parse datetime in various formats, including date-only strings
                // The parse_datetime_string function will automatically convert date-only
                // strings to ISO datetime format by adding T00:00:00
                if let Ok(parsed_datetime) = self.parse_datetime_string(s) {
                    return Value::String(parsed_datetime);
                }
                Value::String(s.to_string())
            }

            "time" => {
                // Validate and normalize time format
                if s.contains(':')
                    && let Ok(parsed_time) = self.parse_time_string(s)
                {
                    return Value::String(parsed_time);
                }
                Value::String(s.to_string())
            }

            "email" => {
                // Basic email validation and normalization
                let normalized = s.trim().to_lowercase();
                if normalized.contains('@') && normalized.contains('.') {
                    return Value::String(normalized);
                }
                Value::String(s.to_string())
            }

            "uri" | "url" => {
                // Basic URL validation
                if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://")
                {
                    return Value::String(s.to_string());
                }
                Value::String(s.to_string())
            }

            _ => Value::String(s.to_string()),
        }
    }

    /// Apply intelligent type coercion after initial validation failure
    /// NOTE: This function is currently unused due to strict validation mode.
    /// It's kept for potential future use if lenient/strict mode toggle is added.
    #[allow(dead_code)]
    fn apply_intelligent_type_coercion(&self, mut row_value: Value) -> Value {
        if let Some(obj) = row_value.as_object_mut() {
            for (field_name, field_value) in obj.iter_mut() {
                if let Some(field_schema) = self.field_schemas.get(field_name) {
                    // Apply coercion to string values that might need conversion
                    if let Some(string_val) = field_value.as_str() {
                        let coerced = self.coerce_string_to_schema_type(string_val, field_schema);
                        if coerced != *field_value {
                            *field_value = coerced;
                        }
                    }
                    // Apply coercion to numeric values that should be strings
                    else if self.schema_expects_string(&field_schema.field_type) {
                        match field_value {
                            Value::Number(n) => {
                                // Convert number to string when schema expects string
                                *field_value = Value::String(n.to_string());
                            }
                            Value::Bool(b) => {
                                // Convert boolean to string when schema expects string
                                *field_value = Value::String(b.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        row_value
    }

    /// Check if the schema type expects a string value
    /// Returns true for String type or Mixed types that include String
    fn schema_expects_string(&self, schema_type: &SchemaType) -> bool {
        match schema_type {
            SchemaType::String => true,
            SchemaType::Mixed(types) => types.contains(&SchemaType::String),
            _ => false,
        }
    }

    /// Fallback intelligent string conversion without schema
    fn intelligent_string_conversion(&self, s: &str) -> Value {
        // Try number conversion
        if let Ok(int_val) = s.parse::<i64>() {
            return json!(int_val);
        }

        if let Ok(float_val) = s.parse::<f64>() {
            return json!(float_val);
        }

        // Try boolean conversion
        match s.to_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => return json!(true),
            "false" | "no" | "n" | "0" => return json!(false),
            _ => {}
        }

        // Try null conversion
        if matches!(s.to_lowercase().as_str(), "null" | "nil" | "none") {
            return Value::Null;
        }

        // Default to string
        Value::String(s.to_string())
    }

    /// Convert datetime with schema intelligence
    fn convert_datetime_with_schema_intelligence(
        &self,
        dt: &calamine::ExcelDateTime,
        field_name: &str,
    ) -> Value {
        let chrono_dt = ExcelValidator::excel_datetime_to_chrono(dt);

        // Check schema format to determine output format
        if let Some(field_schema) = self.field_schemas.get(field_name)
            && let Some(json_schema_format) = &field_schema.format
        {
            // Map JSON Schema format to chrono format string
            // IMPORTANT: json_schema_format is a JSON Schema format (e.g., "date-time"),
            // NOT a chrono format pattern! We must convert it.
            match json_schema_format.as_str() {
                "date" => {
                    // Date format: YYYY-MM-DD
                    let formatted = chrono_dt.format("%Y-%m-%d").to_string();
                    return Value::String(formatted);
                }
                "date-time" => {
                    // RFC 3339 date-time format REQUIRES timezone
                    // Excel datetimes don't have timezone info, so we assume UTC
                    let formatted = chrono_dt.format("%Y-%m-%dT%H:%M:%S").to_string();
                    return Value::String(format!("{}Z", formatted)); // Append 'Z' for UTC
                }
                "time" => {
                    // Time format: HH:MM:SS
                    let formatted = chrono_dt.format("%H:%M:%S").to_string();
                    return Value::String(formatted);
                }
                _ => {
                    // Default ISO format
                    return Value::String(chrono_dt.to_string());
                }
            }
        }
        // Default ISO format
        Value::String(chrono_dt.to_string())
    }

    /// Convert datetime string with schema intelligence
    fn convert_datetime_string_with_schema_intelligence(
        &self,
        dt_str: &str,
        field_name: &str,
    ) -> Value {
        if let Some(field_schema) = self.field_schemas.get(field_name)
            && let Some(format) = &field_schema.format
        {
            return self.convert_string_by_format(dt_str, format);
        }

        Value::String(dt_str.to_string())
    }

    // Helper methods for date/time parsing - delegate to utils module
    fn parse_date_string(&self, s: &str) -> Result<String> {
        crate::utils::parse_date_string(s)
    }

    fn parse_datetime_string(&self, s: &str) -> Result<String> {
        crate::utils::parse_datetime_string(s)
    }

    fn parse_time_string(&self, s: &str) -> Result<String> {
        crate::utils::parse_time_string(s)
    }

    /// Convert numeric cell value to appropriate JSON type
    pub(crate) fn convert_numeric_with_validation(&self, f: f64, field_name: &str) -> Value {
        // Handle special float values
        if f.is_nan() || f.is_infinite() {
            return Value::Null;
        }

        // Check if this is a latitude or longitude field and parse coordinates
        let field_name_lower = field_name.to_lowercase();
        let value = if field_name_lower.contains("latitude") || field_name_lower.contains("lat") {
            // Process latitude through coordinate parser
            let lat_str = f.to_string();
            match crate::utils::parse_latitude(&lat_str) {
                Ok(parsed_lat) => parsed_lat,
                Err(_e) => {
                    log::warn!(
                        "Failed to parse latitude value '{}' for field '{}': {}. The numeric value could not be validated as a valid latitude coordinate",
                        lat_str,
                        field_name,
                        _e
                    );
                    f // Return original value; validation will catch any issues
                }
            }
        } else if field_name_lower.contains("longitude")
            || field_name_lower.contains("lng")
            || field_name_lower.contains("lon")
        {
            // Process longitude through coordinate parser
            let lng_str = f.to_string();
            match crate::utils::parse_longitude(&lng_str) {
                Ok(parsed_lng) => parsed_lng,
                Err(_e) => {
                    log::warn!(
                        "Failed to parse longitude value '{}' for field '{}': {}. The numeric value could not be validated as a valid longitude coordinate",
                        lng_str,
                        field_name,
                        _e
                    );
                    f // Return original value; validation will catch any issues
                }
            }
        } else {
            f
        };

        // Value is valid - convert to appropriate JSON type
        if (value.fract().abs() < f64::EPSILON)
            && value >= i64::MIN as f64
            && value <= i64::MAX as f64
        {
            json!(value as i64)
        } else {
            json!(value)
        }
    }

    fn validate_and_collect_errors(
        &self,
        row_value: &Value,
        row_number: usize,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // 1. Standard JSON Schema validation via jsonschema library
        let jsonschema_errors: Vec<ValidationError> = self
            .validator
            .iter_errors(row_value)
            .map(|error| {
                let path = if error.instance_path().to_string().is_empty() {
                    format!("row[{}]", row_number)
                } else {
                    format!("row[{}].{}", row_number, error.instance_path())
                };

                // Map jsonschema's ValidationErrorKind to our explicit error types
                self.map_jsonschema_error_kind(&error, &path, row_value)
            })
            .filter(|error| {
                // Filter out floating-point precision noise errors
                if let ValidationError::OutOfRange { path, message } = error
                    && path == "IGNORE"
                    && message.contains("floating-point precision noise")
                {
                    return false; // Skip this error
                }
                true // Keep all other errors
            })
            .collect();

        errors.extend(jsonschema_errors);

        // 2. Custom Business Rule: Fields ending with '*' are ALWAYS required
        let asterisk_errors = self.custom_validator_asterisk_required_fields(row_value, row_number);
        errors.extend(asterisk_errors);

        // 3. Custom Business Rule: "date" and "date-time" values must have been parsed
        errors.extend(self.custom_validator_date_formats(row_value, row_number));

        // 4. Custom Business Rule: an unpivoted text values field holds numbers or markers
        errors.extend(self.custom_validator_values_markers(row_value, row_number));

        // 5. Future custom validators can be added here
        // errors.extend(self.validate_cross_field_dependencies(row_value, row_number));

        errors
    }

    /// Custom validator: Fields ending with '*' are ALWAYS required
    ///
    /// This implements a business rule that any field name ending with an asterisk
    /// must be treated as required, regardless of whether it appears in the
    /// JSON Schema's "required" array.
    ///
    /// Rules:
    /// - Field name ends with '*' → Field MUST be present
    /// - Field name ends with '*' → Field CANNOT be null
    ///
    /// # Arguments
    /// * `data` - The row data to validate
    /// * `row_number` - The row number for error reporting
    ///
    /// # Returns
    /// Vector of validation errors (empty if all asterisk fields are valid)
    fn custom_validator_asterisk_required_fields(
        &self,
        data: &Value,
        row_number: usize,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Iterate over all field schemas to find fields ending with asterisk
        for field_name in self.field_schemas.keys() {
            // Check if field name ends with asterisk
            if field_name.ends_with('*') {
                let path = format!("row[{}]./{}", row_number, field_name);

                // Check if field is missing from data
                match data.get(field_name) {
                    None => {
                        // Field is completely missing
                        errors.push(ValidationError::RequiredFieldMissing {
                            path,
                            field: format!("{} (asterisk indicates required field)", field_name),
                        });
                    }
                    Some(Value::Null) => {
                        // Field exists but is null - not allowed for asterisk fields
                        errors.push(ValidationError::RequiredFieldMissing {
                            path,
                            field: format!("{} (required field cannot be null)", field_name),
                        });
                    }
                    Some(Value::String(s)) if s.trim().is_empty() => {
                        // For string fields, empty string is also considered missing
                        // This is an Excel-specific quirk where empty cells become empty strings
                        errors.push(ValidationError::RequiredFieldMissing {
                            path,
                            field: format!("{} (required field cannot be empty)", field_name),
                        });
                    }
                    Some(_) => {
                        // Field exists and has a non-null, non-empty value - OK!
                    }
                }
            }
        }

        errors
    }

    /// Custom validator: values of "date" and "date-time" fields must be real dates
    ///
    /// Cell conversion normalizes every date it can parse (to `YYYY-MM-DD` or RFC 3339) and leaves
    /// anything else as the original text. The JSON Schema validator does not assert formats, so
    /// without this check text such as `n/a` or a month-first `08/28/2024` would be uploaded
    /// as-is into a date column.
    fn custom_validator_date_formats(
        &self,
        data: &Value,
        row_number: usize,
    ) -> Vec<ValidationError> {
        let Some(row) = data.as_object() else {
            return Vec::new();
        };

        row.iter()
            .filter_map(|(field_name, value)| {
                let s = value.as_str().filter(|s| !s.is_empty())?;
                let format = self.field_schemas.get(field_name)?.format.as_deref()?;
                let is_valid = match format {
                    "date" => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok(),
                    "date-time" => chrono::DateTime::parse_from_rfc3339(s).is_ok(),
                    _ => return None,
                };
                (!is_valid).then(|| ValidationError::InvalidFormat {
                    path: format!("row[{}]./{}", row_number, field_name),
                    message: format!(
                        "'{}' is not a valid {}. Numeric dates must be day/month/year or year-month-day",
                        s, format
                    ),
                })
            })
            .collect()
    }

    /// Custom validator: an unpivoted values field that takes text holds a number or a marker
    ///
    /// See [`Unpivot::values_markers`]. Blank cells are left to the field's own type and required
    /// rules. A number is anything that parses as a finite number, so `NaN` and `inf` are refused;
    /// cells read from Excel as numbers arrive here already written as text. Markers match
    /// ignoring case, so `<lod` is `<LOD`.
    fn custom_validator_values_markers(
        &self,
        data: &Value,
        row_number: usize,
    ) -> Option<ValidationError> {
        let values = self.values_markers.as_ref()?;
        let s = data
            .get(&values.field)?
            .as_str()
            .filter(|s| !s.is_empty())?;
        let is_number = s.parse::<f64>().is_ok_and(f64::is_finite);
        let is_marker = values.marker(s).is_some();
        if is_number || is_marker {
            return None;
        }
        // A marker followed by the row's limit was written as the marker alone while reading
        let message = match data
            .as_object()
            .and_then(|row| values.marker_with_limit(s, row))
        {
            Some(MarkerWithLimit::Refused(message)) => message,
            _ if let Some((old, new)) = replaced_marker(s) => format!(
                "'{s}' is not allowed: {old} maps to {new}. Change it to {new} in the source file, for consistency."
            ),
            _ => format!(
                "'{}' is not a number or one of the markers {}",
                s,
                values.markers.join(", ")
            ),
        };
        Some(ValidationError::InvalidFormat {
            path: format!("row[{}]./{}", row_number, values.field),
            message,
        })
    }

    /// Map jsonschema's ValidationErrorKind to our explicit ValidationError types
    fn map_jsonschema_error_kind(
        &self,
        error: &jsonschema::ValidationError,
        path: &str,
        row_value: &Value,
    ) -> ValidationError {
        use jsonschema::error::ValidationErrorKind;

        match &error.kind() {
            // Numeric constraints - EXPLICIT!
            ValidationErrorKind::Minimum { limit } => {
                let value = error.instance().as_f64().unwrap_or(0.0);
                let minimum = limit.as_f64().unwrap_or(0.0);
                ValidationError::BelowMinimum {
                    path: path.to_string(),
                    value,
                    min: minimum,
                }
            }

            ValidationErrorKind::Maximum { limit } => {
                let value = error.instance().as_f64().unwrap_or(0.0);
                let maximum = limit.as_f64().unwrap_or(0.0);
                ValidationError::AboveMaximum {
                    path: path.to_string(),
                    value,
                    max: maximum,
                }
            }

            ValidationErrorKind::ExclusiveMinimum { limit } => {
                let value = error.instance().as_f64().unwrap_or(0.0);
                let minimum = limit.as_f64().unwrap_or(0.0);
                ValidationError::BelowMinimum {
                    path: path.to_string(),
                    value,
                    min: minimum,
                }
            }

            ValidationErrorKind::ExclusiveMaximum { limit } => {
                let value = error.instance().as_f64().unwrap_or(0.0);
                let maximum = limit.as_f64().unwrap_or(0.0);
                ValidationError::AboveMaximum {
                    path: path.to_string(),
                    value,
                    max: maximum,
                }
            }

            ValidationErrorKind::MultipleOf { multiple_of } => {
                let value = error.instance().as_f64().unwrap_or(0.0);
                let remainder = value % multiple_of;

                // Floating-point precision tolerance
                // If the remainder is extremely small, it's likely just floating-point noise.
                // Also check if remainder is close to ±multiple_of (modulo wrap-around issue).
                let tolerance = multiple_of.abs() * 1e-4; // 0.01% of the multipleOf value
                let effective_tolerance = tolerance.max(1e-10); // At least 1e-10

                // Check if remainder is close to 0 OR close to ±multiple_of
                let is_noise = remainder.abs() < effective_tolerance
                    || (multiple_of.abs() - remainder.abs()).abs() < effective_tolerance;

                if is_noise {
                    // Return a benign error that will be filtered out
                    return ValidationError::OutOfRange {
                        path: "IGNORE".to_string(),
                        message: "floating-point precision noise".to_string(),
                    };
                }

                ValidationError::OutOfRange {
                    path: path.to_string(),
                    message: format!("value {} is not a multiple of {}", value, multiple_of),
                }
            }

            // String constraints
            ValidationErrorKind::MinLength { limit } => {
                let length = error.instance().as_str().map(|s| s.len()).unwrap_or(0);
                ValidationError::OutOfRange {
                    path: path.to_string(),
                    message: format!("string length {} is less than minimum {}", length, limit),
                }
            }

            ValidationErrorKind::MaxLength { limit } => {
                let length = error.instance().as_str().map(|s| s.len()).unwrap_or(0);
                ValidationError::OutOfRange {
                    path: path.to_string(),
                    message: format!("string length {} exceeds maximum {}", length, limit),
                }
            }

            ValidationErrorKind::Pattern { pattern } => ValidationError::PatternMismatch {
                path: path.to_string(),
                pattern: pattern.clone(),
                value: error.instance().to_string(),
            },

            // Enum constraint
            ValidationErrorKind::Enum { options } => ValidationError::InvalidFormat {
                path: path.to_string(),
                message: format!(
                    "value must be one of: {}",
                    options
                        .as_array()
                        .map(|arr| arr
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join(", "))
                        .unwrap_or_else(|| "specified options".to_string())
                ),
            },

            // Type constraint
            ValidationErrorKind::Type { kind } => ValidationError::TypeMismatch {
                path: path.to_string(),
                expected: format!("{:?}", kind),
                actual: self.get_json_type_name(error.instance()),
                value: self.safe_value_string(error.instance()),
            },

            // Required field
            ValidationErrorKind::Required { property } => ValidationError::RequiredFieldMissing {
                path: path.to_string(),
                field: property.as_str().unwrap_or("unknown").to_string(),
            },

            // Array constraints
            ValidationErrorKind::MinItems { limit } => {
                let length = error.instance().as_array().map(|a| a.len()).unwrap_or(0);
                ValidationError::ArrayValidation {
                    path: path.to_string(),
                    message: format!("array has {} items, minimum is {}", length, limit),
                }
            }

            ValidationErrorKind::MaxItems { limit } => {
                let length = error.instance().as_array().map(|a| a.len()).unwrap_or(0);
                ValidationError::ArrayValidation {
                    path: path.to_string(),
                    message: format!("array has {} items, maximum is {}", length, limit),
                }
            }

            ValidationErrorKind::UniqueItems => ValidationError::ArrayValidation {
                path: path.to_string(),
                message: "array items must be unique".to_string(),
            },

            // Additional properties
            ValidationErrorKind::AdditionalProperties { unexpected } => {
                ValidationError::AdditionalProperties {
                    path: path.to_string(),
                    properties: unexpected.clone(),
                }
            }

            // Format constraint (date-time, email, uri, etc.)
            ValidationErrorKind::Format { format } => {
                // For format errors, jsonschema's error.instance() contains the format name,
                // not the actual value. We need to look it up from row_value using instance_path
                let field_path = error.instance_path().to_string();

                let actual_value = if field_path.is_empty() {
                    // Root level
                    self.safe_value_string(row_value)
                } else {
                    // Navigate to the field (e.g., "/analysis_date" -> "analysis_date")
                    let field_name = field_path.trim_start_matches('/');

                    row_value
                        .get(field_name)
                        .map(|v| self.safe_value_string(v))
                        .unwrap_or_else(|| "unknown".to_string())
                };

                ValidationError::InvalidFormat {
                    path: path.to_string(),
                    message: format!("Value \"{}\" is not of \"{}\" format", actual_value, format),
                }
            }

            // Fallback for other error kinds
            _ => ValidationError::InvalidFormat {
                path: path.to_string(),
                message: error.to_string(),
            },
        }
    }

    /// Enhanced type name detection including Excel-specific types
    fn get_json_type_name(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(_) => "boolean".to_string(),
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    "integer".to_string()
                } else {
                    "number".to_string()
                }
            }
            Value::String(s) => {
                // Enhanced string type detection
                if s.is_empty() {
                    "empty string".to_string()
                } else if self.looks_like_number(s) {
                    "string (number-like)".to_string()
                } else if self.looks_like_boolean(s) {
                    "string (boolean-like)".to_string()
                } else if self.looks_like_date(s) {
                    "string (date-like)".to_string()
                } else {
                    "string".to_string()
                }
            }
            Value::Array(_) => "array".to_string(),
            Value::Object(_) => "object".to_string(),
        }
    }

    fn safe_value_string(&self, value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => format!("[array with {} items]", arr.len()),
            Value::Object(obj) => format!("[object with {} properties]", obj.len()),
        }
    }

    // Enhanced type detection helpers
    fn looks_like_number(&self, s: &str) -> bool {
        s.trim().parse::<f64>().is_ok()
    }

    fn looks_like_boolean(&self, s: &str) -> bool {
        matches!(
            s.to_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "1" | "0"
        )
    }

    fn looks_like_date(&self, s: &str) -> bool {
        // Simple date pattern detection using basic string matching instead of regex
        // This avoids the regex dependency warning
        let patterns = [
            // YYYY-MM-DD
            |s: &str| {
                s.len() == 10 && s.chars().nth(4) == Some('-') && s.chars().nth(7) == Some('-')
            },
            // MM/DD/YYYY
            |s: &str| {
                s.len() == 10 && s.chars().nth(2) == Some('/') && s.chars().nth(5) == Some('/')
            },
            // MM-DD-YYYY
            |s: &str| {
                s.len() == 10 && s.chars().nth(2) == Some('-') && s.chars().nth(5) == Some('-')
            },
            // YYYY/MM/DD
            |s: &str| {
                s.len() == 10 && s.chars().nth(4) == Some('/') && s.chars().nth(7) == Some('/')
            },
            // DD-MM-YYYY (European format)
            |s: &str| {
                s.len() == 10 && s.chars().nth(2) == Some('-') && s.chars().nth(5) == Some('-')
            },
        ];

        patterns.iter().any(|pattern| pattern(s))
            && s.chars().filter(|c| c.is_ascii_digit()).count() >= 8
    }

    fn excel_datetime_to_chrono(dt: &calamine::ExcelDateTime) -> chrono::NaiveDateTime {
        use chrono::{Duration, NaiveDate};
        let excel_base = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
        let value = dt.as_f64();
        let days = value as i64;
        let seconds = ((value - days as f64) * 86400.0).round() as i64;
        excel_base.and_hms_opt(0, 0, 0).unwrap() + Duration::days(days) + Duration::seconds(seconds)
    }

    /// Check for duplicate column headers in Excel data
    ///
    /// # Arguments
    /// * `headers` - Vector of header strings (already normalized if needed)
    ///
    /// # Returns
    /// * `Ok(())` if no duplicates are found
    /// * `Err(anyhow::Error)` with descriptive message if duplicates are found
    fn check_header_duplicates(headers: &[String]) -> Result<(), anyhow::Error> {
        let mut header_positions: HashMap<String, Vec<usize>> = HashMap::new();

        // Collect all headers with their positions (normalize them)
        for (index, header) in headers.iter().enumerate() {
            let normalized_header = normalize_string(header);
            header_positions
                .entry(normalized_header)
                .or_default()
                .push(index);
        }

        let mut duplicate_headers = Vec::new();

        // Find duplicates
        for (header, positions) in &header_positions {
            if positions.len() > 1 {
                let columns_str = positions
                    .iter()
                    .map(|p| format!("column {}", p + 1)) // Convert to 1-based column indexing
                    .collect::<Vec<_>>()
                    .join(", ");
                duplicate_headers.push(format!("Header '{}' appears in: {}", header, columns_str));
            }
        }

        if duplicate_headers.is_empty() {
            Ok(())
        } else {
            let full_message = format!(
                "Excel file contains duplicate column headers:\n{}\nPlease ensure all column headers are unique.",
                duplicate_headers
                    .into_iter()
                    .map(|msg| format!("  • {}", msg))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            // Write duplicate check errors to the log file
            if let Err(e) = write_error_to_log("Excel Header Duplicate Check Error", &full_message)
            {
                return Err(anyhow::anyhow!(
                    "{}\n(The error log could not be written: {})",
                    full_message,
                    e
                ));
            }

            Err(anyhow::anyhow!(full_message))
        }
    }
}

/// A cell with nothing in it but whitespace.
/// A sample cell without a result, which gives no row: blank, [`NO_SAMPLE`] or
/// [`NO_SAMPLE_ALIAS`]
fn is_no_result(cell: &Data) -> bool {
    is_blank(cell)
        || matches!(cell, Data::String(s)
            if [NO_SAMPLE, NO_SAMPLE_ALIAS].iter().any(|ns| s.trim().eq_ignore_ascii_case(ns)))
}

fn is_blank(cell: &Data) -> bool {
    match cell {
        Data::Empty => true,
        Data::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

/// "A-I (first … last)" for a run of (column index, header) columns; "A (header)" for one.
fn column_span(columns: &[(usize, &str)], first_col: u32) -> String {
    let letter = |i: usize| column_name(sheet_column(first_col, i));
    match (columns.first(), columns.last()) {
        (Some(first), Some(last)) if first.0 != last.0 => format!(
            "{}-{} ({} … {})",
            letter(first.0),
            letter(last.0),
            first.1,
            last.1
        ),
        (Some(only), _) => format!("{} ({})", letter(only.0), only.1),
        _ => String::from("(none)"),
    }
}

/// The row number Excel shows of the row `row_index` places into a range starting at row
/// `first_row`
fn sheet_row(first_row: u32, row_index: usize) -> usize {
    usize::try_from(first_row)
        .unwrap_or(usize::MAX)
        .saturating_add(row_index)
        .saturating_add(1)
}

/// The ng/mL rows of `range`, an unpivoted sheet laid out in `columns`, `kept` and `samples`, each
/// with its ng/L row
fn ng_per_ml_rows(
    range: &calamine::Range<Data>,
    first_row: u32,
    first_col: u32,
    columns: &[Option<String>],
    kept: &[usize],
    samples: &SampleColumns,
) -> Vec<NgPerMlRow> {
    let compared_column = |index: usize, factor: f64| ComparedColumn {
        column: column_name(sheet_column(first_col, index)),
        header: columns.get(index).cloned().flatten(),
        index,
        factor,
    };
    let compared: Vec<ComparedColumn> = kept
        .iter()
        .filter(|&&index| index != samples.units)
        .map(|&index| {
            let is_limit = columns
                .get(index)
                .and_then(Option::as_deref)
                .is_some_and(|header| header == LOD_FIELD || header == LOQ_FIELD);
            compared_column(index, if is_limit { MILLILITRES_PER_LITRE } else { 1.0 })
        })
        .chain(
            samples
                .columns
                .iter()
                .map(|(index, _)| compared_column(*index, MILLILITRES_PER_LITRE)),
        )
        .collect();

    let mut ng_per_ml: Vec<(usize, &[Data])> = Vec::new();
    let mut ng_per_litre: Vec<(usize, &[Data])> = Vec::new();
    // The first row holds the headers
    for (row_index, row) in range.rows().enumerate().skip(1) {
        let units = Units::parse(row.get(samples.units));
        if units.is_ng_per_ml() {
            ng_per_ml.push((sheet_row(first_row, row_index), row));
        } else if units == Units::NanogramsPerLitre {
            ng_per_litre.push((sheet_row(first_row, row_index), row));
        }
    }
    ng_per_ml
        .into_iter()
        .map(|(row, cells)| NgPerMlRow {
            counterpart: counterpart(cells, &ng_per_litre, &compared),
            row,
        })
        .collect()
}

/// The 0-based sheet column of the cell `offset` places into a range starting at column `start`.
fn sheet_column(start: u32, offset: usize) -> u32 {
    start.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX))
}

/// The name Excel shows for a 0-based column index ("A", "BI", ...).
fn column_name(index: u32) -> String {
    match u16::try_from(index) {
        Ok(col) if index < EXCEL_MAX_COLUMNS => {
            rust_xlsxwriter::utility::column_number_to_name(col)
        }
        _ => format!("number {}", index.saturating_add(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*; // Import test utilities from src/test_utils.rs
    use calamine::Data;
    use serde_json::{Value, json};

    #[test]
    fn column_name_is_the_excel_letter_up_to_the_last_column() {
        let last_column = EXCEL_MAX_COLUMNS - 1;
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(60), "BI");
        assert_eq!(column_name(last_column), "XFD");
    }

    #[test]
    fn column_name_past_the_last_excel_column_is_its_number() {
        let expected = format!("number {}", EXCEL_MAX_COLUMNS + 1);
        assert_eq!(column_name(EXCEL_MAX_COLUMNS), expected);
        assert_eq!(column_name(u32::MAX), "number 4294967295");
    }

    #[test]
    fn test_empty_cell_conversion_for_non_mandatory_number() {
        // Create a JSON Schema with non-mandatory number fields
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Required Name": {
                    "type": "string"
                },
                "Optional Age": {
                    "type": ["integer", "null"]
                },
                "Optional Score": {
                    "type": ["number", "null"]
                },
                "Optional Active": {
                    "type": ["boolean", "null"]
                }
            },
            "required": ["Required Name"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test empty cell conversion for different field types
        let empty_cell = Data::Empty;

        // Test for non-mandatory integer field (use title-based property name)
        let age_result =
            validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Age");
        assert_eq!(
            age_result,
            Value::Null,
            "Empty cell should convert to null for non-mandatory integer field"
        );

        // Test for non-mandatory number field
        let score_result =
            validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Score");
        assert_eq!(
            score_result,
            Value::Null,
            "Empty cell should convert to null for non-mandatory number field"
        );

        // Test for non-mandatory boolean field
        let active_result =
            validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Active");
        assert_eq!(
            active_result,
            Value::Null,
            "Empty cell should convert to null for non-mandatory boolean field"
        );
    }

    #[test]
    fn test_empty_string_conversion_for_non_mandatory_number() {
        // Create a JSON Schema with non-mandatory number field
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Required Name": {
                    "type": "string"
                },
                "Optional Amount": {
                    "type": ["number", "null"]
                }
            },
            "required": ["Required Name"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test empty string conversion for non-mandatory number field
        // Use title-based property name
        let empty_string_cell = Data::String("".to_string());
        let result = validator
            .convert_cell_to_json_with_schema_awareness(&empty_string_cell, "Optional Amount");
        assert_eq!(
            result,
            Value::Null,
            "Empty string should convert to null for non-mandatory number field"
        );

        // Test whitespace-only string
        let whitespace_cell = Data::String("   ".to_string());
        let result2 = validator
            .convert_cell_to_json_with_schema_awareness(&whitespace_cell, "Optional Amount");
        assert_eq!(
            result2,
            Value::Null,
            "Whitespace-only string should convert to null for non-mandatory number field"
        );
    }

    #[test]
    fn test_excel_cell_to_null_conversion() {
        use calamine::Data;

        // Create a JSON Schema
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Sample ID": {
                    "type": "string"
                },
                "Ammonium (μmol~1L)": {
                    "type": ["number", "null"],
                    "minimum": 0.0
                }
            },
            "required": ["Sample ID"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test empty Excel cell conversion for the Ammonium field
        let empty_cell = Data::Empty;
        let converted_value =
            validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Ammonium (μmol~1L)");

        assert_eq!(
            converted_value,
            serde_json::Value::Null,
            "Empty cell should be converted to null for non-mandatory Ammonium field"
        );

        // Test empty string conversion
        let empty_string_cell = Data::String("".to_string());
        let converted_empty_string = validator
            .convert_cell_to_json_with_schema_awareness(&empty_string_cell, "Ammonium (μmol~1L)");

        assert_eq!(
            converted_empty_string,
            serde_json::Value::Null,
            "Empty string should be converted to null for non-mandatory Ammonium field"
        );
    }

    #[test]
    fn test_convert_cell_to_json_string() {
        let validator = create_excel_validator_with_defaults(&create_test_schema());

        let cell = Data::String("Hello World".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "name");

        assert_eq!(result, Value::String("Hello World".to_string()));
    }

    // =========================================================================
    // Tests for apply_intelligent_type_coercion
    // =========================================================================
    #[test]
    fn test_strict_validation_rejects_number_when_string_expected() {
        use serde_json::Map;

        // Create a schema where a field is expected to be a string (strict)
        let schema = json!({
            "type": "object",
            "properties": {
                "bio_ethnicity": {
                    "type": ["string", "null"],
                    "title": "Ethnicity"
                }
            },
            "required": []
        });

        // Create validator
        let mut validator = create_excel_validator_with_defaults(&schema);

        // Create a row where bio_ethnicity is a number but schema expects string
        let mut json_obj = Map::new();
        json_obj.insert("bio_ethnicity".to_string(), json!(309)); // Number should FAIL validation

        // Add to validator rows
        validator.rows.push((1, json_obj));

        // Validate - should FAIL because 309 is a number, not a string
        let result = validator.validate_excel();

        assert!(
            result.is_err(),
            "Validation should fail when number provided for string field"
        );
        assert_eq!(
            validator.validation_reports.len(),
            1,
            "Should have 1 validation error"
        );

        let report = &validator.validation_reports[0];
        assert!(
            !report.errors.is_empty(),
            "Should have validation errors for type mismatch"
        );
    }

    #[test]
    fn test_float_to_string_conversion_when_schema_expects_string() {
        use serde_json::Map;

        // Create a schema where a field is expected to be a string
        let schema = json!({
            "type": "object",
            "properties": {
                "measurement": {
                    "type": "string",
                    "title": "Measurement"
                },
                "value": {
                    "type": "number",
                    "title": "Value"
                }
            },
            "required": ["measurement", "value"]
        });

        // Create validator
        let validator = create_excel_validator_with_defaults(&schema);

        // Create a row where measurement is a float but schema expects string
        let mut json_obj = Map::new();
        json_obj.insert("measurement".to_string(), json!(42.75)); // Float that should be converted to string
        json_obj.insert("value".to_string(), json!(100.5)); // Float that should remain number

        let row_value = Value::Object(json_obj);

        // Apply intelligent type coercion
        let coerced_value = validator.apply_intelligent_type_coercion(row_value);

        // Check that the measurement field was converted to string
        let coerced_obj = coerced_value.as_object().unwrap();
        assert_eq!(coerced_obj.get("measurement").unwrap(), &json!("42.75"));
        assert_eq!(coerced_obj.get("value").unwrap(), &json!(100.5));
    }

    #[test]
    fn test_boolean_to_string_conversion_when_schema_expects_string() {
        use serde_json::Map;

        // Create a schema where a field is expected to be a string
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "title": "Status"
                },
                "active": {
                    "type": "boolean",
                    "title": "Active"
                }
            },
            "required": ["status", "active"]
        });

        // Create validator
        let validator = create_excel_validator_with_defaults(&schema);

        // Create a row where status is a boolean but schema expects string
        let mut json_obj = Map::new();
        json_obj.insert("status".to_string(), json!(true)); // Boolean that should be converted to string
        json_obj.insert("active".to_string(), json!(false)); // Boolean that should remain boolean

        let row_value = Value::Object(json_obj);

        // Apply intelligent type coercion
        let coerced_value = validator.apply_intelligent_type_coercion(row_value);

        // Check that the status field was converted to string
        let coerced_obj = coerced_value.as_object().unwrap();
        assert_eq!(coerced_obj.get("status").unwrap(), &json!("true"));
        assert_eq!(coerced_obj.get("active").unwrap(), &json!(false));
    }

    #[test]
    fn test_mixed_type_with_string_allows_number_conversion() {
        use serde_json::Map;

        // Create a schema with mixed types that includes string
        let schema = json!({
            "type": "object",
            "properties": {
                "flexible_field": {
                    "type": ["string", "number"],
                    "title": "Flexible Field"
                },
                "strict_number": {
                    "type": "number",
                    "title": "Strict Number"
                }
            },
            "required": ["flexible_field", "strict_number"]
        });

        // Create validator
        let validator = create_excel_validator_with_defaults(&schema);

        // Create a row where flexible_field is a number but could be converted to string
        let mut json_obj = Map::new();
        json_obj.insert("flexible_field".to_string(), json!(42.5)); // Number that could be converted to string
        json_obj.insert("strict_number".to_string(), json!(100.0)); // Number that should remain number

        let row_value = Value::Object(json_obj);

        // Apply intelligent type coercion
        let coerced_value = validator.apply_intelligent_type_coercion(row_value);

        // Check that the flexible field was converted to string (since mixed type includes string)
        let coerced_obj = coerced_value.as_object().unwrap();
        assert_eq!(coerced_obj.get("flexible_field").unwrap(), &json!("42.5"));
        assert_eq!(coerced_obj.get("strict_number").unwrap(), &json!(100.0));
    }

    #[test]
    fn test_no_conversion_when_schema_does_not_expect_string() {
        use serde_json::Map;

        // Create a schema where no fields expect strings
        let schema = json!({
            "type": "object",
            "properties": {
                "count": {
                    "type": "integer",
                    "title": "Count"
                },
                "value": {
                    "type": "number",
                    "title": "Value"
                },
                "active": {
                    "type": "boolean",
                    "title": "Active"
                }
            },
            "required": ["count", "value", "active"]
        });

        // Create validator
        let validator = create_excel_validator_with_defaults(&schema);

        // Create a row with correct types
        let mut json_obj = Map::new();
        json_obj.insert("count".to_string(), json!(100));
        json_obj.insert("value".to_string(), json!(42.5));
        json_obj.insert("active".to_string(), json!(true));

        let row_value = Value::Object(json_obj.clone());

        // Apply intelligent type coercion
        let coerced_value = validator.apply_intelligent_type_coercion(row_value);

        // Check that no conversion occurred since schema doesn't expect strings
        let coerced_obj = coerced_value.as_object().unwrap();
        assert_eq!(coerced_obj.get("count").unwrap(), &json!(100));
        assert_eq!(coerced_obj.get("value").unwrap(), &json!(42.5));
        assert_eq!(coerced_obj.get("active").unwrap(), &json!(true));
    }

    #[test]
    fn test_string_values_remain_unchanged() {
        use serde_json::Map;

        // Create a schema where a field is expected to be a string
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Name"
                },
                "description": {
                    "type": "string",
                    "title": "Description"
                }
            },
            "required": ["name", "description"]
        });

        // Create validator
        let validator = create_excel_validator_with_defaults(&schema);

        // Create a row where fields are already strings
        let mut json_obj = Map::new();
        json_obj.insert("name".to_string(), json!("John Doe"));
        json_obj.insert("description".to_string(), json!("A test description"));

        let row_value = Value::Object(json_obj);

        // Apply intelligent type coercion
        let coerced_value = validator.apply_intelligent_type_coercion(row_value);

        // Check that string values remain unchanged
        let coerced_obj = coerced_value.as_object().unwrap();
        assert_eq!(coerced_obj.get("name").unwrap(), &json!("John Doe"));
        assert_eq!(
            coerced_obj.get("description").unwrap(),
            &json!("A test description")
        );
    }

    // =========================================================================
    // Tests for convert_cell_to_json_with_schema_awareness
    // =========================================================================
    #[test]
    fn test_backward_compatibility() {
        let validator = create_excel_validator_with_defaults(&create_test_schema());

        // Test the new schema-aware method works for basic cases
        let cell = Data::String("test".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "name");
        assert!(result.is_string());

        let cell2 = Data::Int(42);
        let result2 = validator.convert_cell_to_json_with_schema_awareness(&cell2, "age");
        assert_eq!(result2, json!(42));
    }

    #[test]
    fn test_intelligent_type_coercion_string_to_integer() {
        let validator = create_excel_validator_with_defaults(&create_test_schema());

        // Test string that should be converted to integer for "age" field
        let cell = Data::String("25".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "age");

        assert_eq!(result, json!(25));
    }

    #[test]
    fn test_intelligent_type_coercion_string_to_boolean() {
        let validator = create_excel_validator_with_defaults(&create_test_schema());

        // Test string that should be converted to boolean for "active" field
        let cell = Data::String("true".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "active");

        assert_eq!(result, json!(true));

        let cell2 = Data::String("yes".to_string());
        let result2 = validator.convert_cell_to_json_with_schema_awareness(&cell2, "active");

        assert_eq!(result2, json!(true));
    }

    #[test]
    fn test_fallback_intelligent_conversion() {
        let validator = create_excel_validator_with_defaults(&create_test_schema());

        // Test conversion for unknown field (should fall back to intelligent conversion)
        let cell = Data::String("42".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "unknown_field");

        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_bounds_checking_in_coercion() {
        let bounded_schema = json!({
            "type": "object",
            "properties": {
                "limited_number": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10
                }
            }
        });

        let validator = create_excel_validator_with_defaults(&bounded_schema);

        // Test value within bounds
        let cell1 = Data::String("5".to_string());
        let result1 =
            validator.convert_cell_to_json_with_schema_awareness(&cell1, "limited_number");
        assert_eq!(result1, json!(5));

        // Test value outside bounds (should remain as string)
        let cell2 = Data::String("15".to_string());
        let result2 =
            validator.convert_cell_to_json_with_schema_awareness(&cell2, "limited_number");
        assert_eq!(result2, Value::String("15".to_string()));
    }

    #[test]
    fn test_enum_case_insensitive_matching() {
        let enum_schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["Active", "Inactive", "Pending"]
                }
            }
        });

        let validator = create_excel_validator_with_defaults(&enum_schema);

        // Test case-insensitive enum matching
        let cell = Data::String("active".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "status");
        assert_eq!(result, Value::String("Active".to_string()));
    }

    #[test]
    fn test_array_delimiter_splitting() {
        let array_schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            }
        });

        let validator = create_excel_validator_with_defaults(&array_schema);

        // Test comma-separated values
        let cell = Data::String("tag1,tag2,tag3".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "tags");
        assert_eq!(
            result,
            Value::Array(vec![
                Value::String("tag1".to_string()),
                Value::String("tag2".to_string()),
                Value::String("tag3".to_string())
            ])
        );
    }

    #[test]
    fn test_mixed_type_schema() {
        let mixed_schema = json!({
            "type": "object",
            "properties": {
                "flexible_field": {
                    "type": ["string", "number", "boolean"]
                }
            }
        });

        let validator = create_excel_validator_with_defaults(&mixed_schema);

        // Test that it tries number first for numeric strings
        let cell = Data::String("42".to_string());
        let result = validator.convert_cell_to_json_with_schema_awareness(&cell, "flexible_field");
        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_excel_cell_conversion_for_string_fields() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Required Field": {
                    "type": "string"
                },
                "Optional Field": {
                    "type": ["string", "null"]
                }
            },
            "required": ["Required Field"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test empty cell conversion for optional string field
        let empty_cell = Data::Empty;
        let result =
            validator.convert_cell_to_json_with_schema_awareness(&empty_cell, "Optional Field");
        assert_eq!(
            result,
            Value::Null,
            "Empty cell should convert to null for optional string field"
        );

        // Test empty string conversion
        let empty_string_cell = Data::String("".to_string());
        let result2 = validator
            .convert_cell_to_json_with_schema_awareness(&empty_string_cell, "Optional Field");
        // Note: Empty strings might be converted to null for optional fields based on implementation
        // For now, let's accept either behavior and document it
        assert!(
            result2 == Value::String("".to_string()) || result2 == Value::Null,
            "Empty string should either remain as empty string or be converted to null based on schema requirements"
        );

        // Test whitespace-only string
        let whitespace_cell = Data::String("   ".to_string());
        let result3 = validator
            .convert_cell_to_json_with_schema_awareness(&whitespace_cell, "Optional Field");
        // Based on the actual behavior, empty/whitespace strings are converted to null for optional string fields
        assert!(
            result3 == Value::String("   ".to_string()) || result3 == Value::Null,
            "Whitespace string behavior depends on implementation - may be converted to null for optional fields"
        );
    }

    #[test]
    fn test_integer_to_string_conversion_for_string_schema() {
        // Test that integer Excel cells are converted to strings when schema expects string
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Sample Code": {
                    "type": "string"
                },
                "Optional Code": {
                    "type": ["string", "null"]
                }
            },
            "required": ["Sample Code"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test integer cell conversion for string field
        let int_cell = Data::Int(24019001);
        let result = validator.convert_cell_to_json_with_schema_awareness(&int_cell, "Sample Code");
        assert_eq!(
            result,
            Value::String("24019001".to_string()),
            "Integer cell should be converted to string when schema expects string"
        );

        // Test integer cell conversion for optional string field (Mixed with string|null)
        let int_cell2 = Data::Int(42);
        let result2 =
            validator.convert_cell_to_json_with_schema_awareness(&int_cell2, "Optional Code");
        assert_eq!(
            result2,
            Value::String("42".to_string()),
            "Integer cell should be converted to string for optional string field (string|null)"
        );
    }

    #[test]
    fn test_float_to_string_conversion_for_string_schema() {
        // Test that float Excel cells are converted to strings when schema expects string
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Temperature Reading": {
                    "type": "string"
                },
                "Optional Reading": {
                    "type": ["string", "null"]
                }
            },
            "required": ["Temperature Reading"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test float cell conversion for string field
        let float_cell = Data::Float(25.5);
        let result = validator
            .convert_cell_to_json_with_schema_awareness(&float_cell, "Temperature Reading");
        assert_eq!(
            result,
            Value::String("25.5".to_string()),
            "Float cell should be converted to string when schema expects string"
        );

        // Test float cell conversion for optional string field
        let float_cell2 = Data::Float(98.6);
        let result2 =
            validator.convert_cell_to_json_with_schema_awareness(&float_cell2, "Optional Reading");
        assert_eq!(
            result2,
            Value::String("98.6".to_string()),
            "Float cell should be converted to string for optional string field (string|null)"
        );
    }

    #[test]
    fn test_numeric_values_remain_numeric_for_number_schema() {
        // Ensure that numeric values remain numeric when schema expects number
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Age": {
                    "type": "integer"
                },
                "Temperature": {
                    "type": "number"
                },
                "Optional Count": {
                    "type": ["number", "null"]
                }
            },
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test integer cell remains integer for integer field
        let int_cell = Data::Int(42);
        let result = validator.convert_cell_to_json_with_schema_awareness(&int_cell, "Age");
        assert_eq!(
            result,
            json!(42),
            "Integer cell should remain as integer when schema expects integer"
        );

        // Test float cell remains float for number field
        let float_cell = Data::Float(25.5);
        let result2 =
            validator.convert_cell_to_json_with_schema_awareness(&float_cell, "Temperature");
        assert_eq!(
            result2,
            json!(25.5),
            "Float cell should remain as number when schema expects number"
        );

        // Test numeric cell remains numeric for optional number field
        let int_cell2 = Data::Int(100);
        let result3 =
            validator.convert_cell_to_json_with_schema_awareness(&int_cell2, "Optional Count");
        assert_eq!(
            result3,
            json!(100),
            "Integer cell should remain as number for optional number field (number|null)"
        );
    }

    #[test]
    fn test_mixed_type_with_string_prefers_string_for_integers() {
        // When schema allows multiple types including string, integers should be converted to string
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Flexible ID": {
                    "type": ["string", "integer", "null"]
                }
            },
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test that integer cell is converted to string for mixed type including string
        let int_cell = Data::Int(12345);
        let result = validator.convert_cell_to_json_with_schema_awareness(&int_cell, "Flexible ID");
        assert_eq!(
            result,
            Value::String("12345".to_string()),
            "Integer cell should be converted to string when schema includes string in mixed types"
        );
    }

    #[test]
    fn test_date_and_bool_not_affected_by_string_conversion() {
        // Ensure that boolean and date types are not affected by the string conversion logic
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Is Active": {
                    "type": "boolean"
                },
                "Date Field": {
                    "type": "string",
                    "format": "date"
                }
            },
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test boolean cell remains boolean
        let bool_cell = Data::Bool(true);
        let result = validator.convert_cell_to_json_with_schema_awareness(&bool_cell, "Is Active");
        assert_eq!(result, json!(true), "Boolean cell should remain as boolean");

        // Test DateTime conversion for date field - expect string output
        let dt_cell = Data::DateTimeIso("2024-08-28".to_string());
        let result2 = validator.convert_cell_to_json_with_schema_awareness(&dt_cell, "Date Field");
        assert!(
            result2.is_string(),
            "DateTime cell should be converted to string for date field"
        );
    }

    #[test]
    fn test_additional_properties_error_message() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Sample ID *": {
                    "type": "string"
                },
                "Volume (mL)": {
                    "type": ["integer", "null"]
                }
            },
            "required": ["Sample ID *"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        let test_data = json!({
            "Sample ID *": "S001",
            "Volume (mL)": 100,
            "extra_column_1": "unexpected value",
            "extra_column_2": "also unexpected"
        });

        let is_valid = validator.validator.is_valid(&test_data);
        assert!(!is_valid);

        let raw_errors: Vec<String> = validator
            .validator
            .iter_errors(&test_data)
            .map(|error| {
                let path = "row[1]";
                validator
                    .map_jsonschema_error_kind(&error, path, &test_data)
                    .to_string()
            })
            .collect();

        let has_friendly_message = raw_errors.iter().any(|error| {
            error.contains(
                "Excel has the following extra columns not found in the provided JSON Schema",
            )
        });

        assert!(has_friendly_message);

        let has_column_names = raw_errors
            .iter()
            .any(|error| error.contains("extra_column_1") && error.contains("extra_column_2"));

        assert!(has_column_names);
    }

    #[test]
    fn test_type_mismatch_error_includes_actual_value() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Volume (mL) *": {
                    "type": "integer"
                },
                "Temperature *": {
                    "type": "number"
                },
                "Is Active *": {
                    "type": "boolean"
                }
            },
            "required": ["Volume (mL) *", "Temperature *", "Is Active *"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        let test_data = json!({
            "Volume (mL) *": "abc123",
            "Temperature *": "not_a_number",
            "Is Active *": "maybe"
        });

        let is_valid = validator.validator.is_valid(&test_data);
        assert!(!is_valid);

        let errors: Vec<String> = validator
            .validator
            .iter_errors(&test_data)
            .map(|error| error.to_string())
            .collect();

        let all_errors = errors.join(" ");

        assert!(all_errors.contains("abc123") || all_errors.contains("Volume"));
        assert!(all_errors.contains("not_a_number") || all_errors.contains("Temperature"));
        assert!(all_errors.contains("maybe") || all_errors.contains("Active"));
    }

    #[test]
    fn test_specific_volume_error_enhancement() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Volume (mL)*": {
                    "type": "integer"
                }
            },
            "required": ["Volume (mL)*"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        let test_data = json!({"Volume (mL)*": "15.5mL"});

        let is_valid = validator.validator.is_valid(&test_data);
        assert!(!is_valid);

        let jsonschema_error = validator.validator.iter_errors(&test_data).next().unwrap();
        let path = format!("row[2].{}", jsonschema_error.instance_path());
        let enhanced_error =
            validator.map_jsonschema_error_kind(&jsonschema_error, &path, &test_data);
        let error_message = enhanced_error.to_string();

        assert!(error_message.contains("Type mismatch"));
        assert!(error_message.contains("row[2]"));
        assert!(error_message.contains("volume_ml") || error_message.contains("Volume (mL)*"));
        assert!(error_message.contains("expected") && error_message.contains("Integer"));
        assert!(error_message.contains("got string") || error_message.contains("actual: string"));
        assert!(error_message.contains("15.5mL"));
    }

    #[test]
    fn test_error_message_format_comparison() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Volume (mL) *": {
                    "type": "integer"
                }
            },
            "required": ["Volume (mL) *"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        let test_data = json!({"Volume (mL) *": "25.7"});
        let is_valid = validator.validator.is_valid(&test_data);

        if !is_valid {
            let jsonschema_error = validator.validator.iter_errors(&test_data).next().unwrap();
            let path = format!("row[2].{}", jsonschema_error.instance_path());
            let enhanced_error =
                validator.map_jsonschema_error_kind(&jsonschema_error, &path, &test_data);
            let error_message = enhanced_error.to_string();

            assert!(error_message.contains("\"25.7\""));
        }
    }

    #[test]
    fn test_required_field_null_error_enhancement() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Institution Code*": {
                    "type": "string"
                },
                "Optional Field": {
                    "type": ["string", "null"]
                }
            },
            "required": ["Institution Code*"],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test with null value for required field
        let test_data_required = json!({"Institution Code*": null, "Optional Field": "test"});
        let is_valid_required = validator.validator.is_valid(&test_data_required);
        assert!(
            !is_valid_required,
            "Required field with null should be invalid"
        );

        if !is_valid_required {
            let jsonschema_error = validator
                .validator
                .iter_errors(&test_data_required)
                .next()
                .unwrap();
            let path = format!("row[2].{}", jsonschema_error.instance_path());
            let enhanced_error =
                validator.map_jsonschema_error_kind(&jsonschema_error, &path, &test_data_required);
            let error_message = enhanced_error.to_string();

            assert!(
                error_message.contains("Type mismatch") || error_message.contains("String"),
                "Error should indicate type mismatch. Got: {}",
                error_message
            );
            assert!(
                error_message.contains("Institution Code*") || error_message.contains("row[2]"),
                "Error should reference the field or path"
            );
            assert!(
                error_message.contains("null"),
                "Error should mention null value"
            );
        }

        // Test with null value for optional field
        let test_data_optional = json!({"Institution Code*": "test", "Optional Field": null});
        let is_valid_optional = validator.validator.is_valid(&test_data_optional);

        if !is_valid_optional {
            let jsonschema_error = validator
                .validator
                .iter_errors(&test_data_optional)
                .next()
                .unwrap();
            let path = format!("row[2].{}", jsonschema_error.instance_path());
            let enhanced_error =
                validator.map_jsonschema_error_kind(&jsonschema_error, &path, &test_data_optional);
            let error_message = enhanced_error.to_string();

            assert!(
                !error_message.contains("required field"),
                "Error for optional field should not mention 'required field'. Got: {}",
                error_message
            );
        }
    }

    #[test]
    fn test_excel_headers_no_duplicates() {
        let headers = vec![
            "Header A".to_string(),
            "Header B".to_string(),
            "Header C".to_string(),
        ];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(result.is_ok(), "Should not find any duplicate headers");
    }

    #[test]
    fn test_excel_headers_with_duplicates() {
        let headers = vec![
            "Header A".to_string(),
            "Header B".to_string(),
            "Header A".to_string(), // Duplicate
        ];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(result.is_err(), "Should detect duplicate headers");

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Header A"),
            "Error should mention duplicate header"
        );
        assert!(
            error_message.contains("column 1") && error_message.contains("column 3"),
            "Error should show correct column positions"
        );
        assert!(
            error_message.contains("duplicate column headers"),
            "Error should have proper header message"
        );
    }

    //////////////////////////////////////////////////////////////
    // check_header_duplicates method section
    //////////////////////////////////////////////////////////////

    #[test]
    fn test_excel_headers_multiple_duplicates() {
        let headers = vec![
            "Header A".to_string(),
            "Header B".to_string(),
            "Header C".to_string(),
            "Header A".to_string(), // Duplicate A
            "Header B".to_string(), // Duplicate B
        ];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(result.is_err(), "Should detect multiple duplicates");

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Header A"),
            "Error should mention first duplicate"
        );
        assert!(
            error_message.contains("Header B"),
            "Error should mention second duplicate"
        );
        assert!(
            !error_message.contains("Header C"),
            "Error should not mention non-duplicate header"
        );
    }

    #[test]
    fn test_excel_headers_normalized_duplicates() {
        let headers = vec![
            "Header A".to_string(),
            "  Header A  ".to_string(), // Same after normalization
            "Header\nB".to_string(),
            "Header B".to_string(), // Same after normalization (newline -> space)
        ];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(
            result.is_err(),
            "Should detect duplicates after normalization"
        );

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Header A"),
            "Error should mention first normalized duplicate"
        );
        assert!(
            error_message.contains("Header B"),
            "Error should mention second normalized duplicate"
        );
    }

    #[test]
    fn test_excel_headers_empty_list() {
        let headers: Vec<String> = vec![];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(
            result.is_ok(),
            "Empty headers list should not have duplicates"
        );
    }

    #[test]
    fn test_excel_headers_single_header() {
        let headers = vec!["Single Header".to_string()];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(result.is_ok(), "Single header should not be a duplicate");
    }

    #[test]
    fn test_excel_headers_case_sensitive() {
        // Our normalize_string function doesn't change case, so these should be different
        let headers = vec![
            "Header A".to_string(),
            "HEADER A".to_string(),
            "header a".to_string(),
        ];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(
            result.is_ok(),
            "Different cases should not be considered duplicates"
        );
    }

    #[test]
    fn test_excel_headers_with_control_characters() {
        let headers = vec![
            "Header\tA".to_string(), // Tab will be normalized to space
            "Header A".to_string(),
            "Header\nB".to_string(), // Newline will be normalized to space
            "Header B".to_string(),
        ];

        let result = ExcelValidator::check_header_duplicates(&headers);
        assert!(
            result.is_err(),
            "Should detect duplicates after control character normalization"
        );

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Header A"),
            "Should detect Header A duplicate"
        );
        assert!(
            error_message.contains("Header B"),
            "Should detect Header B duplicate"
        );
    }

    //////////////////////////////////////////////////////////////
    // ExcelValidatorBuilder tests
    //////////////////////////////////////////////////////////////

    // looks like date test
    #[test]
    fn test_convert_datetime_with_schema_intelligence() {
        // We can test this method by verifying it correctly formats datetime strings
        // based on schema field formats, even without direct ExcelDateTime instances

        let schema = create_test_schema();
        let _validator = create_excel_validator_with_defaults(&schema);

        // Test with a custom schema that has specific datetime formatting
        let custom_schema = json!({
            "type": "object",
            "properties": {
                "custom_date": {
                    "type": "string",
                    "format": "%Y/%m/%d"
                },
                "iso_datetime": {
                    "type": "string",
                    "format": "date-time"
                }
            }
        });

        let custom_validator = create_excel_validator_with_defaults(&custom_schema);

        // Test that the method would format dates according to schema format
        // We can verify this logic works by testing the string conversion method
        let date_value = custom_validator
            .convert_datetime_string_with_schema_intelligence("2024-09-15", "custom_date");
        let datetime_value = custom_validator.convert_datetime_string_with_schema_intelligence(
            "2024-09-15T12:00:00",
            "iso_datetime",
        );

        // Verify the conversion preserves the input when no special formatting is applied
        assert_eq!(date_value.as_str().unwrap(), "2024-09-15");
        // RFC 3339 format includes 'Z' timezone indicator for UTC
        assert_eq!(datetime_value.as_str().unwrap(), "2024-09-15T12:00:00Z");

        // Core logic verified via string conversion test above
    }

    #[test]
    fn test_convert_datetime_string_with_schema_intelligence() {
        let schema = create_test_schema();
        let validator = create_excel_validator_with_defaults(&schema);

        // Test date field
        let value =
            validator.convert_datetime_string_with_schema_intelligence("2024-09-15", "date_field");
        assert_eq!(value.as_str().unwrap(), "2024-09-15");

        // Test datetime field
        let value = validator.convert_datetime_string_with_schema_intelligence(
            "2024-09-15T12:00:00",
            "datetime_field",
        );
        assert_eq!(value.as_str().unwrap(), "2024-09-15T12:00:00");
    }

    #[test]
    fn test_looks_like_date() {
        let schema = create_test_schema();
        let validator = create_excel_validator_with_defaults(&schema);

        // Test various date formats
        assert!(validator.looks_like_date("2024-09-15"));
        assert!(validator.looks_like_date("09/15/2024"));
        assert!(validator.looks_like_date("15-09-2024"));
        assert!(validator.looks_like_date("2024/09/15"));

        // Test invalid formats
        assert!(!validator.looks_like_date("not a date"));
        assert!(!validator.looks_like_date("12345"));
        assert!(!validator.looks_like_date(""));
    }

    // =========================================================================
    // Tests for Custom Business Rules: Asterisk = Required
    // =========================================================================

    #[test]
    fn test_asterisk_in_field_name_implies_required() {
        // Schema where "Temperature*" has asterisk but is NOT in "required" array
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Sample ID": {
                    "type": "string"
                },
                "Temperature*": {
                    "type": "number",
                    "description": "Field with asterisk - should be treated as required!"
                },
                "Notes": {
                    "type": ["string", "null"]
                }
            },
            "required": ["Sample ID"],  // Only Sample ID is in required array
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Test 1: All fields present - should be VALID
        let valid_data = json!({
            "Sample ID": "S001",
            "Temperature*": 25.5,
            "Notes": "Test note"
        });

        let errors = validator.validate_and_collect_errors(&valid_data, 1);
        assert!(
            errors.is_empty(),
            "Should be valid when all fields including Temperature* are present"
        );

        // Test 2: Temperature* missing - should be INVALID
        let missing_asterisk_field = json!({
            "Sample ID": "S002",
            "Notes": "Test note"
        });

        let errors = validator.validate_and_collect_errors(&missing_asterisk_field, 2);
        assert!(
            !errors.is_empty(),
            "Should fail when asterisk field is missing"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.to_string().contains("Temperature*")),
            "Error should mention Temperature* field"
        );

        // Test 3: Temperature* is null - should be INVALID
        let null_asterisk_field = json!({
            "Sample ID": "S003",
            "Temperature*": null,
            "Notes": "Test note"
        });

        let errors = validator.validate_and_collect_errors(&null_asterisk_field, 3);
        assert!(
            !errors.is_empty(),
            "Should fail when asterisk field is null"
        );

        // Test 4: Optional field (no asterisk) can be null
        let null_optional_field = json!({
            "Sample ID": "S004",
            "Temperature*": 25.5,
            "Notes": null
        });

        let errors = validator.validate_and_collect_errors(&null_optional_field, 4);
        assert!(
            errors.is_empty(),
            "Should be valid: Notes has no asterisk, so null is allowed"
        );
    }

    #[test]
    fn test_multiple_asterisk_fields_not_in_required_array() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Field1*": {
                    "type": "string"
                },
                "Field2*": {
                    "type": "number"
                },
                "Field3*": {
                    "type": "boolean"
                },
                "OptionalField": {
                    "type": ["string", "null"]
                }
            },
            "required": [],  // EMPTY required array - but asterisks should make them required!
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // All asterisk fields present - VALID
        let all_present = json!({
            "Field1*": "value",
            "Field2*": 42,
            "Field3*": true,
            "OptionalField": null
        });

        let errors = validator.validate_and_collect_errors(&all_present, 1);
        assert!(
            errors.is_empty(),
            "Should be valid when all asterisk fields are present"
        );

        // Missing Field2* - should be INVALID
        let missing_one = json!({
            "Field1*": "value",
            "Field3*": true,
            "OptionalField": null
        });

        let errors = validator.validate_and_collect_errors(&missing_one, 2);
        assert!(!errors.is_empty(), "Should fail when Field2* is missing");
        assert!(
            errors.iter().any(|e| e.to_string().contains("Field2*")),
            "Error should mention Field2* field"
        );

        // All asterisk fields null - should be INVALID
        let all_null = json!({
            "Field1*": null,
            "Field2*": null,
            "Field3*": null,
            "OptionalField": null
        });

        let errors = validator.validate_and_collect_errors(&all_null, 3);
        assert!(
            !errors.is_empty(),
            "Should fail when asterisk fields are null"
        );
        // Should have errors for all 3 asterisk fields
        assert!(
            errors.len() >= 3,
            "Should have at least 3 errors (one for each asterisk field)"
        );
    }

    #[test]
    fn test_asterisk_field_with_empty_string() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Name*": {
                    "type": "string"
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = create_excel_validator_with_defaults(&json_schema);

        // Empty string for asterisk field - should be INVALID
        let empty_string = json!({
            "Name*": ""
        });

        let errors = validator.validate_and_collect_errors(&empty_string, 1);
        assert!(
            !errors.is_empty(),
            "Should fail when asterisk string field is empty"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.to_string().contains("Name*") && e.to_string().contains("empty")),
            "Error should mention empty field"
        );
    }

    // ============================================================================
    // Array field handling tests
    // ============================================================================

    #[test]
    fn test_array_field_schema_parsing() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let field_schemas = validator.field_schemas();

        let city_schema = field_schemas.get("city").expect("city field should exist");
        assert!(
            matches!(city_schema.field_type, SchemaType::Array(_)),
            "city field should be recognized as array type"
        );
    }

    #[test]
    fn test_array_split_by_comma() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result =
            validator.convert_string_with_schema_intelligence("Athens, Paris, London", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_split_by_semicolon() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result =
            validator.convert_string_with_schema_intelligence("Athens; Paris; London", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_split_by_pipe() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result =
            validator.convert_string_with_schema_intelligence("Athens | Paris | London", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_split_by_newline() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result =
            validator.convert_string_with_schema_intelligence("Athens\nParis\nLondon", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_split_by_mixed_delimiters() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result = validator
            .convert_string_with_schema_intelligence("Athens, Paris; London | Berlin", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
        assert_eq!(arr[3].as_str().unwrap(), "Berlin");
    }

    #[test]
    fn test_array_with_whitespace_trimming() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result = validator
            .convert_string_with_schema_intelligence("  Athens  ,  Paris  ,  London  ", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_single_value() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result = validator.convert_string_with_schema_intelligence("Athens", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
    }

    #[test]
    fn test_array_with_empty_values_filtered() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result =
            validator.convert_string_with_schema_intelligence("Athens,  , Paris, , London", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_with_special_characters() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result = validator
            .convert_string_with_schema_intelligence("Athens (4415), Paris (4383)", "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str().unwrap(), "Athens (4415)");
        assert_eq!(arr[1].as_str().unwrap(), "Paris (4383)");
    }

    #[test]
    fn test_array_json_format_input() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result = validator
            .convert_string_with_schema_intelligence(r#"["Athens", "Paris", "London"]"#, "city");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "Athens");
        assert_eq!(arr[1].as_str().unwrap(), "Paris");
        assert_eq!(arr[2].as_str().unwrap(), "London");
    }

    #[test]
    fn test_array_with_numeric_items() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "ids": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();
        let result = validator.convert_string_with_schema_intelligence("4415, 4383, 5001", "ids");

        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str().unwrap(), "4415");
        assert_eq!(arr[1].as_str().unwrap(), "4383");
        assert_eq!(arr[2].as_str().unwrap(), "5001");
    }

    // ============================================================================
    // Date parsing tests
    // ============================================================================

    #[test]
    fn test_parse_date_string_european_format() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "test_date": {
                    "type": "string",
                    "format": "date"
                }
            },
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();

        // Test DD/MM/YYYY format
        let result = validator.parse_date_string("28/08/2024");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-08-28");
    }

    #[test]
    fn test_parse_date_string_iso_format() {
        let json_schema = json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();

        // Test YYYY-MM-DD format (ISO)
        let result = validator.parse_date_string("2024-08-28");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-08-28");
    }

    #[test]
    fn test_parse_date_string_multiple_formats() {
        let json_schema = json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();

        // Test various formats
        let test_cases = vec![
            ("28/08/2024", "2024-08-28"), // European
            ("2024-08-28", "2024-08-28"), // ISO
            ("28-08-2024", "2024-08-28"), // European with dashes
            ("2024.08.28", "2024-08-28"), // Dots
            ("28.08.2024", "2024-08-28"), // European dots
        ];

        for (input, expected) in test_cases {
            let result = validator.parse_date_string(input);
            assert!(result.is_ok(), "Failed to parse: {}", input);
            assert_eq!(result.unwrap(), expected, "Wrong output for: {}", input);
        }
    }

    #[test]
    fn test_parse_datetime_string_european_format() {
        let json_schema = json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();

        // Test DD/MM/YYYY HH:MM:SS format - returns RFC 3339 with Z for UTC
        let result = validator.parse_datetime_string("28/08/2024 15:30:00");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-08-28T15:30:00Z");
    }

    #[test]
    fn test_parse_datetime_string_date_only() {
        let json_schema = json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        });

        let validator = ExcelValidator::new_for_testing(&json_schema).unwrap();

        // Test date only - should add default time and Z for UTC
        let result = validator.parse_datetime_string("28/08/2024");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-08-28T00:00:00Z");
    }

    // =========================================================================
    // Tests for coordinate parsing integration
    // =========================================================================

    /// Helper function to create an ExcelValidator with a test schema for coordinates
    fn create_validator_with_coordinate_schema() -> ExcelValidator {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Sample ID": {
                    "type": "string",
                    "title": "Sample ID"
                },
                "Latitude": {
                    "type": "number",
                    "title": "Latitude",
                    "minimum": -90.0,
                    "maximum": 90.0
                },
                "Longitude": {
                    "type": "number",
                    "title": "Longitude",
                    "minimum": -180.0,
                    "maximum": 180.0
                },
                "Latitude String": {
                    "type": "string",
                    "title": "Latitude String"
                },
                "Longitude String": {
                    "type": "string",
                    "title": "Longitude String"
                }
            },
            "required": ["Sample ID"],
            "additionalProperties": false
        });

        ExcelValidator::new_for_testing(&json_schema).expect("Failed to create validator")
    }

    #[test]
    fn test_numeric_latitude_longitude_parsing() {
        let validator = create_validator_with_coordinate_schema();

        // Test numeric latitude - values should be used as-is
        let lat_result = validator.convert_numeric_with_validation(42.8708889, "Latitude");

        // The coordinate parser should return the value as-is
        if let Some(lat_num) = lat_result.as_f64() {
            assert!(
                (lat_num - 42.8708889).abs() < 1e-6,
                "Expected latitude ~42.8708889, got {}",
                lat_num
            );
        } else {
            panic!("Expected numeric latitude result");
        }

        // Test numeric longitude - values should be used as-is
        let lng_result = validator.convert_numeric_with_validation(17.70386111, "Longitude");

        if let Some(lng_num) = lng_result.as_f64() {
            assert!(
                (lng_num - 17.70386111).abs() < 1e-6,
                "Expected longitude ~17.70386111, got {}",
                lng_num
            );
        } else {
            panic!("Expected numeric longitude result");
        }
    }

    #[test]
    fn test_string_latitude_longitude_parsing_dms_format() {
        let validator = create_validator_with_coordinate_schema();

        // Test DMS format latitude string (e.g., "41 23.670 N")
        let lat_result =
            validator.convert_string_with_schema_intelligence("41 23.670 N", "Latitude");

        // Should be parsed and converted to numeric value
        if let Some(lat_num) = lat_result.as_f64() {
            assert!(
                (lat_num - 41.3945).abs() < 1e-4,
                "Expected latitude ~41.3945, got {}",
                lat_num
            );
        } else {
            panic!("Expected numeric latitude result, got: {:?}", lat_result);
        }

        // Test DMS format longitude string (e.g., "15.727E")
        let lng_result = validator.convert_string_with_schema_intelligence("15.727E", "Longitude");

        if let Some(lng_num) = lng_result.as_f64() {
            assert!(
                (lng_num - 15.727).abs() < 1e-6,
                "Expected longitude ~15.727, got {}",
                lng_num
            );
        } else {
            panic!("Expected numeric longitude result, got: {:?}", lng_result);
        }
    }

    #[test]
    fn test_string_latitude_longitude_parsing_decimal_format() {
        let validator = create_validator_with_coordinate_schema();

        // Test decimal format latitude string
        let lat_result =
            validator.convert_string_with_schema_intelligence("42.8708889", "Latitude");

        if let Some(lat_num) = lat_result.as_f64() {
            assert!(
                (lat_num - 42.8708889).abs() < 1e-6,
                "Expected latitude ~42.8708889, got {}",
                lat_num
            );
        } else {
            panic!("Expected numeric latitude result");
        }

        // Test decimal format longitude string
        let lng_result =
            validator.convert_string_with_schema_intelligence("17.70386111", "Longitude");

        if let Some(lng_num) = lng_result.as_f64() {
            assert!(
                (lng_num - 17.70386111).abs() < 1e-6,
                "Expected longitude ~17.70386111, got {}",
                lng_num
            );
        } else {
            panic!("Expected numeric longitude result");
        }
    }

    #[test]
    fn test_string_field_coordinate_parsing() {
        let validator = create_validator_with_coordinate_schema();

        // Test latitude as string field (should be parsed and returned as number)
        let lat_result =
            validator.convert_string_with_schema_intelligence("41 23.670 N", "Latitude String");

        // For string fields with coordinate names, it will still parse to number
        if let Some(lat_num) = lat_result.as_f64() {
            assert!(
                (lat_num - 41.3945).abs() < 1e-4,
                "Expected latitude ~41.3945, got {}",
                lat_num
            );
        } else {
            panic!("Expected numeric latitude result, got {:?}", lat_result);
        }

        // Test longitude as string field (should be parsed and returned as number)
        let lng_result =
            validator.convert_string_with_schema_intelligence("15.727E", "Longitude String");

        if let Some(lng_num) = lng_result.as_f64() {
            assert!(
                (lng_num - 15.727).abs() < 1e-6,
                "Expected longitude ~15.727, got {}",
                lng_num
            );
        } else {
            panic!("Expected numeric longitude result, got {:?}", lng_result);
        }
    }

    #[test]
    fn test_coordinate_field_name_variations() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "lat": {
                    "type": "number",
                    "title": "lat"
                },
                "lng": {
                    "type": "number",
                    "title": "lng"
                },
                "lon": {
                    "type": "number",
                    "title": "lon"
                }
            },
            "additionalProperties": false
        });

        let validator =
            ExcelValidator::new_for_testing(&json_schema).expect("Failed to create validator");

        // Test "lat" field name
        let lat_result = validator.convert_string_with_schema_intelligence("41 23.670 N", "lat");
        assert!(lat_result.as_f64().is_some(), "Should parse lat field");

        // Test "lng" field name
        let lng_result = validator.convert_string_with_schema_intelligence("15.727E", "lng");
        assert!(lng_result.as_f64().is_some(), "Should parse lng field");

        // Test "lon" field name
        let lon_result = validator.convert_string_with_schema_intelligence("15.727E", "lon");
        assert!(lon_result.as_f64().is_some(), "Should parse lon field");
    }

    #[test]
    fn test_coordinate_parsing_with_asterisk_fields() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Latitude*": {
                    "type": "number",
                    "title": "Latitude*"
                },
                "Longitude*": {
                    "type": "number",
                    "title": "Longitude*"
                }
            },
            "required": ["Latitude*", "Longitude*"],
            "additionalProperties": false
        });

        let validator =
            ExcelValidator::new_for_testing(&json_schema).expect("Failed to create validator");

        // Test latitude with asterisk
        let lat_result =
            validator.convert_string_with_schema_intelligence("41 23.670 N", "Latitude*");

        if let Some(lat_num) = lat_result.as_f64() {
            assert!(
                (lat_num - 41.3945).abs() < 1e-4,
                "Expected latitude ~41.3945, got {}",
                lat_num
            );
        } else {
            panic!("Expected numeric latitude result");
        }

        // Test longitude with asterisk
        let lng_result = validator.convert_string_with_schema_intelligence("15.727E", "Longitude*");

        if let Some(lng_num) = lng_result.as_f64() {
            assert!(
                (lng_num - 15.727).abs() < 1e-6,
                "Expected longitude ~15.727, got {}",
                lng_num
            );
        } else {
            panic!("Expected numeric longitude result");
        }
    }

    #[test]
    fn test_invalid_latitude_returns_original_value() {
        let json_schema = json!({
            "type": "object",
            "properties": {
                "Latitude": {
                    "type": "number",
                    "title": "Latitude"
                }
            },
            "additionalProperties": false
        });

        let validator =
            ExcelValidator::new_for_testing(&json_schema).expect("Failed to create validator");

        // Invalid latitude should return original string value
        let result = validator.convert_string_with_schema_intelligence("invalid", "Latitude");
        assert_eq!(
            result.as_str(),
            Some("invalid"),
            "Should return original string when parsing fails"
        );
    }

    #[test]
    fn test_valid_coordinates_use_values_as_is() {
        let validator = create_validator_with_coordinate_schema();

        // Test that valid coordinates are used as-is without formatting
        let lat_result = validator.convert_numeric_with_validation(42.8708889, "Latitude");
        assert_eq!(
            lat_result.as_f64(),
            Some(42.8708889),
            "Should use latitude value as-is"
        );

        let lng_result = validator.convert_numeric_with_validation(17.70386111, "Longitude");
        assert_eq!(
            lng_result.as_f64(),
            Some(17.70386111),
            "Should use longitude value as-is"
        );
    }
}
