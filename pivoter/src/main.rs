/*
    Unpivot/untranspose Excel columns usage example:
    rm *.xlsx;
    cargo build -r &&  ./target/release/unpivoter  \
    --excel-file file.xlsx \
    --sheet "Sheet1" \
    --separator-column 11
*/

use calamine::{Data, Reader, Xlsx, open_workbook};
use clap::Parser;
use std::collections::HashSet;
use std::error::Error;
use std::path::PathBuf;
use unpivoter::{cell_to_string, detect_bool_columns, sanitize_filename, write_data};

#[derive(Parser, Debug)]
#[command(name = "pivoter")]
#[command(about = "Unpivot/untranspose Excel columns")]
struct Args {
    /// Path to the Excel file (.xlsx)
    #[arg(short, long)]
    excel_file: PathBuf,

    /// Sheet name to process
    #[arg(short, long)]
    sheet: String,

    /// Column index (0-based) that separates ID columns from columns to unpivot.
    /// Columns 0..separator_column stay as-is, columns separator_column..separator_column + 1 get unpivoted.
    #[arg(short = 'c', long)]
    separator_column: usize,

    /// Comma-separated list of column indices (0-based) to treat as boolean.
    /// Values 0/1 in these columns will be converted to FALSE/TRUE.
    /// If not specified, boolean columns are auto-detected.
    #[arg(short = 'b', long, value_delimiter = ',')]
    bool_columns: Option<Vec<usize>>,

    /// Print debug info about cell types being read
    #[arg(long, default_value = "false")]
    debug: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut workbook: Xlsx<_> = open_workbook(&args.excel_file).map_err(|e| {
        format!(
            "Failed to open workbook: {} with error: {}",
            args.excel_file.display(),
            e
        )
    })?;

    let range = workbook
        .worksheet_range(&args.sheet)
        .map_err(|e| format!("Failed to read sheet '{}': {}", args.sheet, e))?;

    let mut rows = range.rows();

    // First row is headers (as strings for column names)
    let header_row = rows.next().ok_or("Empty sheet - no header row")?;
    let headers: Vec<String> = header_row.iter().map(cell_to_string).collect();

    // Collect all data rows for analysis
    let data_rows: Vec<Vec<Data>> = rows.map(|row| row.to_vec()).collect();

    // Determine boolean columns
    let bool_cols: HashSet<usize> = if let Some(ref explicit_cols) = args.bool_columns {
        explicit_cols.iter().copied().collect()
    } else {
        // Auto-detect boolean columns
        detect_bool_columns(&data_rows, &headers, args.debug)
    };

    if args.debug && !bool_cols.is_empty() {
        let col_names: Vec<_> = bool_cols
            .iter()
            .map(|&i| {
                headers
                    .get(i)
                    .map(|s| format!("[{}] {}", i, s))
                    .unwrap_or_else(|| format!("[{}]", i))
            })
            .collect();
        eprintln!("DEBUG: Boolean columns detected: {:?}", col_names);
    }

    let id_cols = args.separator_column;
    let val_start = args.separator_column;

    // Create output Excel workbook
    let mut out_workbook = rust_xlsxwriter::Workbook::new();
    let worksheet = out_workbook.add_worksheet();
    worksheet.set_name("Analytical results")?;

    // Write output header: ID columns + "Sample Tag" + "_value_"
    let mut col: u16 = 0;
    for header in headers.iter().take(id_cols) {
        worksheet.write_string(0, col, header)?;
        col += 1;
    }
    worksheet.write_string(0, col, "Sample Tag")?;
    worksheet.write_string(0, col + 1, "_value_")?;

    // Process data rows
    let mut out_row: u32 = 1;
    let mut debug_printed = false;
    for row in &data_rows {
        // Debug: print types of first data row
        if args.debug && !debug_printed {
            eprintln!("DEBUG: First data row cell types:");
            for (i, cell) in row.iter().enumerate() {
                let header = headers.get(i).map(String::as_str).unwrap_or("?");
                eprintln!("  [{}] {}: {:?}", i, header, cell);
            }
            debug_printed = true;
        }

        for col_idx in val_start..row.len() {
            // Write ID columns (preserving types, with bool normalization)
            for (i, cell) in row.iter().take(id_cols).enumerate() {
                let is_bool_col = bool_cols.contains(&i);
                write_data(worksheet, out_row, i as u16, cell, is_bool_col)?;
            }

            // Write Attribute (header name)
            let attr = headers.get(col_idx).map(String::as_str).unwrap_or("");
            worksheet.write_string(out_row, id_cols as u16, attr)?;

            // Write Value (preserving original type)
            if let Some(cell) = row.get(col_idx) {
                let is_bool_col = bool_cols.contains(&col_idx);
                write_data(worksheet, out_row, id_cols as u16 + 1, cell, is_bool_col)?;
            }

            out_row += 1;
        }
    }

    // Autofit column widths based on content
    worksheet.autofit();

    let output_path = PathBuf::from(format!(
        "{}_{}.xlsx",
        sanitize_filename(&args.excel_file.to_string_lossy()),
        sanitize_filename(&args.sheet)
    ));
    out_workbook.save(&output_path)?;
    println!(
        "Successfully unpivoted '{}' sheet '{}' -> '{}'",
        args.excel_file.display(),
        args.sheet,
        output_path.display()
    );
    Ok(())
}
