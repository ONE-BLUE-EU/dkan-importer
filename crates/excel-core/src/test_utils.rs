// Test utilities available to both unit and integration tests
// Only compiled when testing

use crate::excel_validator::ExcelValidator;
use crate::utils::normalize_string;
use serde_json::{Value, json};

/// [LEGACY] Helper function to normalize DKAN field data for testing
#[allow(dead_code)]
pub fn normalize_field_data_for_tests(json_schema: Value) -> Result<Value, anyhow::Error> {
    let mut normalized = json_schema.clone();

    if let Some(fields) = normalized.get_mut("fields").and_then(|f| f.as_array_mut()) {
        for field in fields.iter_mut() {
            if let Some(field_obj) = field.as_object_mut() {
                // Normalize title if present
                if let Some(title) = field_obj.get("title").and_then(|t| t.as_str()) {
                    field_obj.insert("title".to_string(), json!(normalize_string(title)));
                }
            }
        }
    }

    Ok(normalized)
}

/// Factory function with default test file paths
#[allow(dead_code)]
pub fn create_excel_validator_with_defaults(schema: &Value) -> ExcelValidator {
    ExcelValidator::new_for_testing(schema).unwrap()
}

/// Creates a basic test schema for testing purposes
#[allow(dead_code)]
pub fn create_test_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "minLength": 1
            },
            "age": {
                "type": "integer",
                "minimum": 0,
                "maximum": 150
            },
            "email": {
                "type": "string",
                "format": "email"
            },
            "active": {
                "type": "boolean"
            },
            "score": {
                "type": "number",
                "minimum": 0.0,
                "maximum": 100.0
            }
        },
        "required": ["name", "age", "email"],
        "additionalProperties": false
    })
}

/// A cell to write into a test workbook
pub enum Cell {
    Text(&'static str),
    Number(f64),
    Empty,
    /// An Excel error value such as "#N/A", stored as the cached result of a formula
    Error(&'static str),
    /// A formula whose cached result is this text, as `IF(A1="","",A1*1000)` gives `""`
    FormulaText(&'static str),
}

/// The bytes of a new .xlsx workbook with `rows` (the first row being the headers) in a sheet named
/// "Sheet1". Nothing is written to disk.
pub fn test_workbook(rows: &[Vec<Cell>]) -> Vec<u8> {
    use rust_xlsxwriter::{Formula, Workbook};

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Sheet1").unwrap();
    // A formula with an empty result is stored as `t="str"` and `<v></v>`, as Excel does, not as
    // the writer's default result of 0
    worksheet.set_formula_result_default("");
    for (row_idx, row) in rows.iter().enumerate() {
        for (col_idx, cell) in row.iter().enumerate() {
            let (r, c) = (row_idx as u32, col_idx as u16);
            match cell {
                Cell::Text(s) => {
                    worksheet.write_string(r, c, *s).unwrap();
                }
                Cell::Number(n) => {
                    worksheet.write_number(r, c, *n).unwrap();
                }
                Cell::Empty => {}
                Cell::Error(e) => {
                    worksheet
                        .write_formula(r, c, Formula::new("=NA()").set_result(*e))
                        .unwrap();
                }
                Cell::FormulaText(text) => {
                    worksheet
                        .write_formula(r, c, Formula::new("=\"\"").set_result(*text))
                        .unwrap();
                }
            }
        }
    }
    workbook.save_to_buffer().unwrap()
}
