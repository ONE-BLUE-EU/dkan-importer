pub fn write(path: &str, headers: &[String], rows: &[Vec<String>]) -> Result<(), anyhow::Error> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();

    // Write headers at row 0
    sheet.write_row(0, 0, headers)?;

    // Write data rows starting from row 1
    for (row_idx, row) in rows.iter().enumerate() {
        for (col_idx, cell) in row.iter().enumerate() {
            sheet.write_string((row_idx + 1) as u32, col_idx as u16, cell)?;
        }
    }

    workbook.save(path)?;
    Ok(())
}
