use calamine::Data;
use rust_xlsxwriter::{Format, Worksheet};
use std::collections::HashSet;

/// Sanitize a string for use in a filename
pub fn sanitize_filename(s: &str) -> String {
    s.replace(".xlsx", "")
        .replace(
            [
                ' ', '(', ')', '.', '-', '/', ':', ';', ',', '!', '?', '&', '\'', '"', '`', '~',
                '@', '#', '$', '%', '^',
            ],
            "_",
        )
        .to_lowercase()
}

/// Convert a calamine Data cell to a String
pub fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                (*f as i64).to_string()
            } else {
                f.to_string()
            }
        }
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{:?}", e),
    }
}

/// Detect columns that contain only boolean-like values (Bool, 0, 1, or Empty)
pub fn detect_bool_columns(rows: &[Vec<Data>], headers: &[String], debug: bool) -> HashSet<usize> {
    if rows.is_empty() {
        return HashSet::new();
    }

    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut is_bool_col = vec![true; num_cols];
    let mut has_actual_bool = vec![false; num_cols]; // Must have at least one Bool value

    for row in rows {
        for (col_idx, cell) in row.iter().enumerate() {
            if col_idx >= num_cols {
                continue;
            }

            match cell {
                Data::Empty => {
                    // Empty is compatible with boolean columns
                }
                Data::Bool(_) => {
                    // Actual boolean - this column is definitely boolean
                    has_actual_bool[col_idx] = true;
                }
                Data::Int(i) if *i == 0 || *i == 1 => {
                    // 0 or 1 integer is compatible with boolean
                }
                Data::Float(f) if *f == 0.0 || *f == 1.0 => {
                    // 0.0 or 1.0 float is compatible with boolean
                }
                _ => {
                    // Any other value means this is not a boolean column
                    is_bool_col[col_idx] = false;
                }
            }
        }
    }

    // A column is boolean if it only has bool-like values AND has at least one actual Bool
    let result: HashSet<usize> = (0..num_cols)
        .filter(|&i| is_bool_col[i] && has_actual_bool[i])
        .collect();

    if debug && !result.is_empty() {
        for &col_idx in &result {
            let header = headers.get(col_idx).map(String::as_str).unwrap_or("?");
            eprintln!(
                "DEBUG: Auto-detected boolean column [{}] '{}'",
                col_idx, header
            );
        }
    }

    result
}

/// Write a calamine Data cell to Excel
/// If `as_bool` is true, numeric 0/1 values will be converted to FALSE/TRUE
pub fn write_data(
    worksheet: &mut Worksheet,
    row: u32,
    col: u16,
    data: &Data,
    as_bool: bool,
) -> Result<(), rust_xlsxwriter::XlsxError> {
    match data {
        Data::Empty => {}
        Data::String(s) => {
            worksheet.write_string(row, col, s)?;
        }
        Data::Int(i) => {
            if as_bool {
                worksheet.write_boolean(row, col, *i != 0)?;
            } else {
                worksheet.write_number(row, col, *i as f64)?;
            }
        }
        Data::Float(f) => {
            if as_bool && (*f == 0.0 || *f == 1.0) {
                worksheet.write_boolean(row, col, *f == 1.0)?;
            } else {
                worksheet.write_number(row, col, *f)?;
            }
        }
        Data::Bool(b) => {
            worksheet.write_boolean(row, col, *b)?;
        }
        Data::DateTime(dt) => {
            // Excel stores dates as serial numbers; without a number format they display as such
            let num_format = if dt.is_duration() {
                "[h]:mm:ss"
            } else if dt.as_f64().fract() == 0.0 {
                "yyyy-mm-dd"
            } else {
                "yyyy-mm-dd hh:mm:ss"
            };
            let format = Format::new().set_num_format(num_format);
            worksheet.write_number_with_format(row, col, dt.as_f64(), &format)?;
        }
        Data::DateTimeIso(s) => {
            worksheet.write_string(row, col, s)?;
        }
        Data::DurationIso(s) => {
            worksheet.write_string(row, col, s)?;
        }
        Data::Error(e) => {
            worksheet.write_string(row, col, format!("#{:?}", e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{ExcelDateTime, ExcelDateTimeType, Reader, Xlsx};
    use std::io::Cursor;

    /// Write one cell with `write_data`, save the workbook, and read the cell back
    fn round_trip(data: &Data, as_bool: bool) -> Data {
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let worksheet = workbook.add_worksheet();
        write_data(worksheet, 0, 0, data, as_bool).unwrap();
        let buffer = workbook.save_to_buffer().unwrap();

        let mut reader = Xlsx::new(Cursor::new(buffer)).unwrap();
        let range = reader.worksheet_range("Sheet1").unwrap();
        range.get((0, 0)).cloned().unwrap_or(Data::Empty)
    }

    #[test]
    fn dates_are_written_as_dates_not_serial_numbers() {
        // 2024-01-01, and 2024-01-01 12:00
        for serial in [45292.0, 45292.5] {
            let cell = Data::DateTime(ExcelDateTime::new(
                serial,
                ExcelDateTimeType::DateTime,
                false,
            ));

            match round_trip(&cell, false) {
                Data::DateTime(dt) => {
                    assert!(dt.is_datetime(), "{dt:?}");
                    assert_eq!(dt.as_f64(), serial);
                }
                other => panic!("expected a date cell for {serial}, got {other:?}"),
            }
        }
    }

    #[test]
    fn numbers_stay_numbers() {
        assert_eq!(
            round_trip(&Data::Float(45292.0), false),
            Data::Float(45292.0)
        );
    }

    #[test]
    fn zero_one_values_become_booleans_only_in_boolean_columns() {
        assert_eq!(round_trip(&Data::Int(1), true), Data::Bool(true));
        assert_eq!(round_trip(&Data::Float(0.0), true), Data::Bool(false));
        assert_eq!(round_trip(&Data::Int(1), false), Data::Float(1.0));
    }

    #[test]
    fn boolean_columns_need_an_actual_boolean_and_only_boolean_like_values() {
        let headers = vec![
            "bool".to_string(),
            "zero_one".to_string(),
            "mixed".to_string(),
        ];
        let rows = vec![
            vec![Data::Bool(true), Data::Int(0), Data::Bool(false)],
            vec![Data::Int(0), Data::Int(1), Data::Int(2)],
            vec![Data::Empty, Data::Float(1.0), Data::Empty],
        ];

        let detected = detect_bool_columns(&rows, &headers, false);

        assert_eq!(detected, HashSet::from([0]));
    }
}
