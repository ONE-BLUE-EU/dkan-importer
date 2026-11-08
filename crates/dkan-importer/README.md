# DKAN Importer

A Rust application for validating Excel files against DKAN data dictionaries and importing the data into DKAN datasets. This tool fetches data dictionaries from DKAN API endpoints, validates Excel files against the converted JSON schemas, and uploads valid data as CSV distributions to DKAN datasets.

## Features

- 🌐 **DKAN Integration**: Fetches data dictionaries directly from DKAN API endpoints
- 📋 **Excel File Reading**: Supports `.xlsx`, `.xlsm`, `.xls` files using the calamine crate
- 🔄 **Schema Conversion**: Automatically converts DKAN data dictionaries to JSON Schema format
- 🔍 **Smart Validation**: Validates each Excel row with intelligent type coercion and error reporting
- 📊 **CSV Export**: Exports validated data to CSV format with schema-aware date formatting
- 🚀 **Automated Upload**: Uploads CSV files to DKAN and adds them as dataset distributions
- 🔐 **Authentication**: Supports username/password authentication for DKAN API access
- 🚨 **Error Reporting**: Detailed error logging with row-by-row validation reports

## Installation

Make sure you have Rust installed, then clone and build the project:

```bash
git clone <repository-url>
cd dkan-importer
cargo build --release
```

## Usage

### Basic Usage

```bash
cargo run -- --base-url <DKAN_URL> --excel-file <EXCEL_FILE> --data-dictionary-id <UUID> --username <USERNAME> --dataset-id <DATASET_UUID>
```

### Examples

```bash
# Basic usage with password prompt
cargo run -- --base-url https://dkan.example.com --excel-file ./data/sample-data.xlsx --data-dictionary-id "12345678-1234-5678-9012-123456789012" --username admin --dataset-id "87654321-4321-8765-2109-876543210987"

# Specify sheet name and password
cargo run -- --base-url https://dkan.example.com --excel-file ./data/sample-data.xlsx --sheet-name "Sample" --data-dictionary-id "12345678-1234-5678-9012-123456789012" --username admin --password mypassword --dataset-id "87654321-4321-8765-2109-876543210987"

# Using the built binary
./target/release/dkan-importer --base-url https://dkan.example.com --excel-file data.xlsx --data-dictionary-id "uuid-here" --username admin --dataset-id "dataset-uuid-here"
```

### Command Line Arguments

- **`--base-url`** (required) - Base URL of the DKAN instance (must be HTTPS)
- **`--excel-file`** (required) - Path to the Excel file to validate and import
- **`--data-dictionary-id`** (required) - UUID of the DKAN data dictionary to use for validation
- **`--username`** (required) - Username for DKAN API authentication
- **`--password`** (optional) - Password for authentication (will be prompted if not provided)
- **`--dataset-id`** (required) - UUID of the existing DKAN dataset to add the CSV as a distribution
- **`--sheet-name`** (optional) - Name of the Excel sheet to process (defaults to "Sheet1"). Give it more than once to merge several sheets into one CSV file. See [Merging several sheets](#merging-several-sheets)
- **`--unpivot`** (optional) - Unpivot the sheet: one row per sample cell. Requires `--headers-column`. See [Unpivoting a wide sheet](#unpivoting-a-wide-sheet)
- **`--headers-column`** (with `--unpivot` only) - The data dictionary field that receives each sample column's header
- **`--values-column`** (with `--unpivot` only) - The data dictionary field that receives each sample cell's value (defaults to "Concentration")
- **`--values-markers`** (with `--unpivot` only) - The text a sample cell may hold instead of a number when the values column is a text field (defaults to `<LOD` `<LOQ` `<MLOD` `<MLOQ` `X`). See [Markers instead of numbers](#markers-instead-of-numbers)
- **`--marker-limits`** (with `--unpivot` only) - The markers that may be followed by their limit, as `MARKER=FIELD` (defaults to `"<LOD=Limit of detection"` `"<LOQ=Limit of quantification"` `"<MLOD=Limit of detection"` `"<MLOQ=Limit of quantification"`). See [A marker followed by its limit](#a-marker-followed-by-its-limit)
- **`--extra-columns-to [FIELD]`** (optional, not with `--unpivot`) - Move the columns that are not data dictionary fields into FIELD as JSON instead of refusing them (FIELD defaults to "Remark 1"). See [Keeping extra columns](#keeping-extra-columns)

### Unpivoting a wide sheet

Some sheets have one column per sample, for example "Analytical results": each row is a substance, and each sample column holds its concentration in that sample. `--unpivot` turns every sample cell into its own row:

```bash
dkan-importer ... --sheet-name "Analytical results" \
  --unpivot --headers-column "Sample Tag" --values-column "Concentration"
```

```
Substance name | ST01_Surface | ST02_Surface
Okadaic acid   | 0.88         | 0.40
```

becomes

```
Substance name | Sample Tag   | Concentration
Okadaic acid   | ST01_Surface | 0.88
Okadaic acid   | ST02_Surface | 0.40
```

- The leading columns whose headers are data dictionary fields are copied to every row. Every column after them is a sample column.
- A blank sample cell is no result, so it gives no row. Cells with only spaces, and formulas that give `""`, count as blank.
- The sheet must have an "Identification Level" column before the sample columns, and the data dictionary that field; otherwise the run stops before reading any rows. In every row with results, the cell must hold a number, uploaded as written, or be blank, uploaded as `2` and listed in the run log: `row 3, column B "Identification Level": Cell value: blank. Imported as "2", because a blank Identification Level is taken as 2.`. The importer does not rewrite anything else, text included, even text reading a number, which Excel shows the same: it stops the run before anything is written or uploaded, listing each row to correct in the source file with its cell value:

  ```
  The sheet "Analytical results" has 2 row(s) whose Identification Level is not a number. A blank one is taken as 2: correct these cells in the source file and try again:
    row 4: Cell value: "2A"
    row 5: Cell value: "2"
  ```
- `--headers-column` and `--values-column` are only accepted with `--unpivot`, and `--unpivot` needs `--headers-column`. Both must be data dictionary fields that are not columns in the sheet. Otherwise the run stops and lists the fields that can be used.
- The run also stops, before anything is written or uploaded, if the sheet does not start with data dictionary fields, has no sample columns, or has a data dictionary field among the sample columns.
- Before validating, it prints what it will do, for example: `Keeping 9 column(s) A-I (Anal. Method Serial No. … Limit of quantification); unpivoting 74 column(s) J-CE (ST01_Surface … ST41_Bottom) into Sample Tag / Concentration: 518 row(s); 4 blank sample cell(s) skipped.` The blank cells are only mentioned when there are some.
- Validation errors name the source row. The row data in the error log includes the sample column's header.

#### Markers instead of numbers

Some results are not numbers: `<LOD` (below the limit of detection), `<LOQ` (below the limit of quantification), `<MLOD` and `<MLOQ` (the same, from labs reporting against the method's limits, which are then the row's limits) or `X` (detected, in a Peak Area row). The limits themselves are in their own columns, so these markers are kept as written. That needs a values column that is a text field (`string`) in the data dictionary. A number field refuses them as type mismatches.

A text field would accept any text, so the importer checks every non-blank sample cell: it must be a number or one of the `--values-markers`. Anything else, such as `<LDO` or `n.d.`, is a validation error at its source row:

```
Invalid format at row[2]./Concentration: '<LDO' is not a number or one of the markers <LOD, <LOQ
```

- The default markers are `<LOD`, `<LOQ`, `<MLOD`, `<MLOQ` and `X`. Giving `--values-markers` replaces them, so list all of them: `--values-markers "<LOD" "<LOQ" "<MLOD" "<MLOQ" "X" "<MDL"`. A marker other than these five has no detection characterisation yet, so its cells give no row and a question in the run log.
- `NS` (no sample) is no result, so a sample cell holding it gives no row, with any values field, like a blank cell. `ND` is treated the same as `NS`. The summary line counts them: `...: 518 row(s); 12 NS or ND (no sample) cell(s) skipped.`, and the run log lists each one. For the same reason neither can be one of the `--values-markers`.
- `<LQ` and `NQ` are refused, even when given in `--values-markers`: they map to `<LOQ`, so change them in the source file, for consistency. A sample cell with either (any case, also with a limit, as in `<LQ:0.25`) is a validation error saying so: `'<LQ' is not allowed: <LQ maps to <LOQ. Change it to <LOQ in the source file, for consistency.`
- Markers match ignoring case and are written uppercase, so `<lod` is uploaded as `<LOD`, also when given in `--values-markers` or `--marker-limits`. Spaces around a cell's text are ignored, so `<LOQ ` is `<LOQ`.
- Numbers are uploaded as Excel stores them (`0.880999866251062`).
- With a number field the markers are not used, because the field already refuses all text.
- The summary line ends with the rule, for example `... 518 row(s). Concentration takes numbers or <LOD, <LOQ, <MLOD, <MLOQ, X.`

#### A marker followed by its limit

Some sheets write the limit after the marker, as in `<LOQ:0.25`, although the same row already has it in its "Limit of quantification" column. When the number after the marker equals the row's limit, the cell is uploaded as the marker alone (`<LOQ`), the same as in sheets without the number. Spaces around the `:` are ignored, and numbers compare as numbers, so `<LOQ : 0.250` matches a limit of `0.25`.

A different number, or a row whose limit is blank, is a validation error, so no number is dropped that the row does not keep:

```
Invalid format at row[2]./Concentration: '<LOQ:0.3' does not end with the row's Limit of quantification (0.25); write '<LOQ' or '<LOQ:0.25'
Invalid format at row[3]./Concentration: '<LOQ:0.25' has a number, but the row has no Limit of quantification
```

- The default limits are `"<LOD=Limit of detection"`, `"<LOQ=Limit of quantification"`, `"<MLOD=Limit of detection"` and `"<MLOQ=Limit of quantification"`. Giving `--marker-limits` replaces them. A marker without a limit takes no number.
- A limit for a marker that is not one of the `--values-markers` does not apply. Its field must be a data dictionary field, or the run stops before reading any rows.
- The summary line counts the cells written as the marker alone, for example `... Concentration takes numbers or <LOD, <LOQ, <MLOD, <MLOQ, X. 375 value(s) written as their marker alone, the number after it being the row's limit: <LOD (Limit of detection), <LOQ (Limit of quantification), <MLOD (Limit of detection), <MLOQ (Limit of quantification).`

#### Detection characterisation

Each result's "Detection characterisation" is worked out from its row's "Concentration units" (matched ignoring case and spaces) and limits, replacing the sheet's own value, which describes the whole substance row rather than each result. The data dictionary must have "Concentration units", "Detection characterisation", "Limit of detection" and "Limit of quantification", and the sheet a "Concentration units" column before the sample columns; otherwise the run stops before reading any rows. If the sheet has no "Detection characterisation" column, one is added.

| Units | Result | Detection characterisation |
| ----- | ------ | -------------------------- |
| a concentration | a number at least the limit of quantification | detected quantitatively |
| a concentration | a number from the limit of detection up to the limit of quantification, or `<LOQ` or `<MLOQ` | detected, not quantified |
| a concentration | a number below the limit of detection, or `<LOD` or `<MLOD` | not detected |
| a concentration, the row without a limit of quantification | a number not below the limit of detection, if any | detected quantitatively (asked about) |
| Peak Area | `X` | detected qualitatively |
| none | `X` | detected qualitatively (taken as Peak Area; the units stay empty) |
| items | a whole number, 0 or more | detected quantitatively |

- The concentration units are ng/L, ng/mL, ng/m3, µg/m3, ng/g and µg/g, matched ignoring case and spaces, with µ also written as μ or u, and m3 as m³. They are imported as written.
- A number is compared with the limits its row has. In a row with only a limit of quantification, a number below it is "detected, not quantified". In a row without a limit of quantification, a number not below the limit of detection (or any number, without either limit) is imported as "detected quantitatively", and the run log asks the data owners to confirm it. `<LOD` and `<LOQ` keep their meaning whatever the limits.
- Where the result's characterisation differs from the sheet's own, the run log says so, with the cell value and the reason, and the question it depends on, if any: `row 2, column AQ "24019033": Cell value: "<LOQ". Imported with Detection characterisation "detected, not quantified" instead of the sheet's "detected quantitatively", because "<LOQ" means detected, but below the Limit of quantification. See Q1.`, or `...: Cell value: 0.05. Imported with Detection characterisation "not detected" instead of the sheet's "detected quantitatively", because the value is below the Limit of detection (0.1).`
- Some sheets hold rows in one unit beside rows calculated from them by formulas in another: ng/mL beside ng/L, µg/m3 beside ng/m3, µg/g beside ng/g, each x1000. A row in ng/mL, µg/m3 or µg/g is skipped when a row in ng/L, ng/m3 or ng/g, respectively, holds its values: the same text (ignoring case and spaces), and its numbers x1000 in the limits and the sample columns, to 6 significant digits. Where it is blank or holds an Excel error, the other row may hold anything (such as a Classification). The importer reads formulas as the values Excel saved, so the source file needs no change. Otherwise the row is imported as written, in its own units, naming the row in the other units differing from it in the fewest cells, if any, as a formula there may point at the wrong row. The summary counts the skipped rows, and the run log lists each such row:

  ```
  3 row(s) in units also held converted (ng/mL as ng/L, µg/m3 as ng/m3, µg/g as ng/g): 1 skipped, as another row holds their values, and 2 imported as written, as none does:
    row 2: skipped: in ng/mL; its values are in the ng/L row 23, which is imported
    row 18: imported in ng/mL, as no ng/L row has its values (x1000 in the limits and the sample columns); the closest, row 39, differs in 1 cell(s), first at row 39, column J "Limit of quantification": Cell value: blank; expected 0.6746666666666667
    row 24: imported in µg/g, as the sheet has no ng/g row to hold its values
  ```

  A skipped row's identification level is not checked, as it is not imported.
- What is not clear yet is not imported: the cell gives no row, and the run log lists it with the number of its question (`... Not imported, because "X" is a Peak Area result, but the row is in ng/L. See Q2.`). The questions are printed and logged once each, numbered, with the number of cells:

  ```
  Questions to data owners:
  Q1 (14 cells): Results between the Limit of detection and the Limit of quantification, and "<LOQ" and "<MLOQ" results, are imported as "detected, not quantified". The template's dropdown list does not have this value: it has "detected semiquantitatively". Which should be used?
  Q2 (3 cells): X results in rows with concentration units (ng/L, ng/mL, ng/m3, µg/m3, ng/g, µg/g) are not imported, as X is a Peak Area result. Are the units or the results wrong?
  ```

  The questions are: a number taken as quantified in a row without a limit of quantification (imported), a number in a Peak Area row, `X` in a concentration row (X is a Peak Area result), an items result that is not a whole number of 0 or more, other units (such as ug/kg), any other result without units or with a marker that has no characterisation. "detected, not quantified" is also asked about, as the template's dropdown list has "detected semiquantitatively" instead.

### Keeping extra columns

By default a sheet column that is not a data dictionary field stops the run ("Excel has the following extra columns not found in the provided JSON Schema"). `--extra-columns-to` keeps that data instead, as JSON in one text field:

```bash
dkan-importer ... --sheet-name "Sample" --extra-columns-to "Remark 3"
```

```
Sample Tag | Remark 3    | Slice num | DNA (Yes/No)
ST01-a     |             | 1         | yes
ST01-b     | Super dirty | 2         |
```

becomes

```
Sample Tag | Remark 3
ST01-a     | {"Slice num":1,"DNA (Yes/No)":"yes"}
ST01-b     | {"Remark 3":"Super dirty","Slice num":2}
```

- Given without a field, `--extra-columns-to` uses "Remark 1".
- The field must be a data dictionary field that takes text; otherwise the run stops and lists the fields that can be used. If it is not a sheet column, it is added to the export.
- Keys are the column headers, in sheet order. Numbers stay numbers, text stays text, and blank cells are left out.
- Text already in the field is kept in the object under the field's own header, first. Rows without extra data keep the field as it is; they never get `{}`.
- The JSON is validated like any other value, so a field with a maximum length refuses rows whose JSON is longer.
- Data in a column without a header is still refused.
- Before validating, it prints what it will do, for example: `Moving 2 extra column(s) into Remark 3: column C "Slice num", column D "DNA (Yes/No)"; 2 row(s) have extra data.`

### Merging several sheets

Some workbooks split one kind of data over several sheets, for example the analytical results of each matrix: "Analytical results (column wate", "Analytical results (Air)", "Analytical results (Sediment)", ... Give `--sheet-name` once for each sheet to upload them together, as one CSV file and one distribution:

```bash
dkan-importer ... --unpivot --headers-column "Sample Tag" \
  --sheet-name "Analytical results (column wate" \
  --sheet-name "Analytical results (Air)" \
  --sheet-name "Analytical results (Sediment)"
```

- Each sheet is read, unpivoted and validated on its own, exactly as if it were imported alone: its run log lines name its own rows and columns, and an ng/mL row, for example, is only matched with an ng/L row of the same sheet. In the output and the run log, each sheet's lines come under `Sheet "<name>":`.
- The CSV file then holds the rows of every sheet, one sheet after the other in the order given, under one header row.
- Every sheet must have the same columns, in the same order, once read (for an unpivoted sheet: the same leading columns; the sample columns may differ). Otherwise the run stops, before validating any rows, naming the first sheet that differs and both lists of columns.
- Every sheet is validated before the run stops, so one run lists everything to correct. Each sheet refused gets a line in the error, as in `Sheet "Analytical results (Air)": Excel validation failed with 1 error(s) across 1 row(s). Check the error log for details.`, and its own entry in the error log, titled `Excel Validation Error Report: sheet "<name>"`. Nothing is uploaded unless every sheet is accepted.
- A sheet given twice stops the run before it starts.
- The top of the run log has one `Excel sheet:` line for each sheet, in the order given.
- Before validating, it prints how many rows the merge holds: `✅ Merging 3 sheets into one CSV file: 1234 row(s)`.
- The CSV file and the run log are named after the merge; see [CSV Output](#csv-output).

## DKAN Data Dictionary Format

The application fetches data dictionaries from DKAN using the metastore API:
```
GET /api/1/metastore/schemas/data-dictionary/items
```

The data dictionary is automatically converted to JSON Schema format for validation. DKAN field types are mapped as follows:

- `integer` → JSON Schema `integer`
- `number`/`float` → JSON Schema `number`
- `boolean` → JSON Schema `boolean`
- `datetime` → JSON Schema `string` with `date-time` format
- `array` → JSON Schema `array`
- `object` → JSON Schema `object`
- Everything else → JSON Schema `string`

## Excel File Format

- The first row should contain column headers that match the data dictionary field names
- Each subsequent row represents a data record to validate
- Empty cells are converted to `null` values
- The application performs intelligent type coercion based on the schema
- Numeric dates are read day-first (`03/04/2024` is 3 April). Month-first dates such as `08/28/2024`, and any other text in a date column, are validation errors
- Cells containing Excel error values (`#VALUE!`, `#N/A`, `#DIV/0!`, ...) are failed calculations, not data, so they are read as blank cells (in a numeric column, that means the placeholder below). Before validating, the importer lists every such cell:

  ```
  ⚠️  Emptied 2 Excel error value(s), uploading them as blank cells:
    row 3, column B "Depth": #N/A
    row 7, column K "ST02": #VALUE!
  ```
- Empty cells in numeric columns are uploaded as `000000000000.000000` (number) or `0` (integer), so that DKAN creates numeric columns

Example Excel structure:
```
| sample_id | collection_date | latitude | longitude | temperature |
| --------- | --------------- | -------- | --------- | ----------- |
| SAMPLE001 | 2024-01-15      | 45.123   | 12.456    | 18.5        |
| SAMPLE002 | 2024-01-16      | 45.124   | 12.457    | 19.2        |
```

## Workflow

1. **URL and file**: Refuses a base URL that is not https, then reads the Excel file, before asking for the password
2. **Authentication**: Checks that DKAN accepts the login for uploads, then reads the dataset's name for the run log
3. **Schema Retrieval**: Fetches the specified data dictionary from DKAN
4. **Schema Conversion**: Converts DKAN data dictionary to JSON Schema
5. **Validation**: Validates each Excel row against the schema; refused rows stop the run before anything is uploaded
6. **CSV Export**: Saves valid data to a timestamped CSV file in the current directory, beside the run log; a CSV file that cannot be saved stops the run before the upload
7. **Upload**: Uploads the CSV file to DKAN's custom importer endpoint
8. **Distribution**: Adds the uploaded CSV as a distribution to the specified dataset, replacing the one its data dictionary describes, whose file is then deleted

## Code layout and tests

`main.rs` is the shell: it parses the arguments, names the run's files, and gives the import its environment (the file system, the password prompt, DKAN over HTTP, the error log file). The import itself, `import::run`, reaches all of these through the `Environment` and `Dkan` traits, so `tests/import_test.rs` runs whole imports against fakes, with no network, files or terminal. `HttpDkan` builds each request and reads each answer with pure functions, tested in `src/dkan/tests.rs`; only its `send` does I/O. `tests/cli_test.rs` runs the real binary against a closed local port.

## Output

### Console Output
- The summary of what was read (columns kept and unpivoted, rows, skipped cells), and the questions about the data
- The error that stopped the run, if any, on the standard error
- Last, in every run, whether it succeeded or stopped, what became of the CSV file and of the upload, also at the end of the run log:

  ```
  ✅ CSV file saved: analytical_results_(i)_9e94380f-73d5-4655-9a0f-519adbff655c_2026-11-07_14-52-21.csv (3312 row(s))
  ✅ Uploaded analytical_results_(i)_9e94380f-73d5-4655-9a0f-519adbff655c_2026-11-07_14-52-21.csv and added it to dataset "Arctic Ocean - Arctic Ocean - 2023" (9e94380f-73d5-4655-9a0f-519adbff655c) as a new distribution
  ```

  The CSV file is saved in the current directory, beside the run log, before it is uploaded under the same name, so the file sent is kept even when the upload fails. A run that stops ends instead with `❌ CSV file not generated` and `❌ Nothing uploaded`; one that stops later says how far it got: the CSV generated but not saved (`❌ CSV file generated (3312 row(s)), but not saved as ...`, and nothing uploaded), saved but not uploaded (`❌ Nothing uploaded`), uploaded to its URL but not added to the dataset (`❌ Uploaded ... to <url>, but not added to dataset ...`), or added in place of a distribution whose file could not be deleted (`⚠️  Uploaded ..., replacing old.csv, whose file could not be deleted`). When it replaces one, the line says so: `..., replacing old.csv, whose file was deleted`.

### Run Log File
Every run writes a log in the current directory, named exactly like the run's CSV file but ending in `.run.log`: `<sheet>_<dataset>_<YYYY-MM-DD_HH-MM-SS>.run.log`, lowercase, with the sheet name's spaces as `_`, as in `analytical_results_(i)_some-dataset_2026-11-05_10-00-00.run.log` (for a merge, see [CSV Output](#csv-output)). Each run writes its own log; the newest describes the latest run. It is written even when the run stops, and never contains the password. It holds:
- First, what the run was given, with the dataset's and the data dictionary's names (their titles in DKAN, read once the login is accepted), then two blank lines:

  ```
  Excel file: Sample_Collection&Results_Arctic 1.xlsx
  Excel sheet: Analytical results (I)
  Dataset Name: Arctic Ocean - Arctic Ocean - 2023
  Dataset ID: 9e94380f-73d5-4655-9a0f-519adbff655c
  Dictionary Name: <the data dictionary's title>
  Dictionary ID: 6bc71c4e-a63b-436b-98c1-b74363fb482e
  ```

  When a name is not read, the reason takes its place: `(not read: the run stopped before reaching DKAN)` for a run stopped by its URL, its file or the login, or DKAN's error. A dataset whose name cannot be read does not stop the run; a data dictionary that cannot be read does, once the top of the log is written.
- Every line the run printed, including the error that stopped it, and last what became of the CSV file and of the upload
- Every change made to a cell instead of taking it as written, one line per cell with its value and its reason, so the data can be checked and corrected at its source:

```
3 cell(s) not imported exactly as written:
  row 2, column E "ST01": Cell value: "<loq:0.25". Imported as "<LOQ", because the number after the marker is the row's Limit of quantification.
  row 2, column F "ST02": Cell value: "ns". Not imported, because no sample was taken.
  row 3, column E "ST01": Cell value: #VALUE! (Excel error). Imported as a blank cell, because an Excel error is a failed calculation.
```

Every line naming a cell gives its value the same way: text in quotes, exactly as written, spaces included, so that text reading a number (`"0.3"`) cannot be taken for the number (`0.3`); `blank` for an empty cell; and an Excel error as `#VALUE! (Excel error)`.

Blank sample cells are not listed, as a sheet has thousands: the summary line counts them.

Rows that fail validation are not changes: they are listed in the run's error log.

Problems that must be corrected in the source file (an Identification Level that is not a number, data in a column without a header) stop the run only once every row has been read. So one run reports all of them together, after the summary, the changes and the questions, and nothing is written or uploaded.

### Error Log File
When rows fail validation, the run writes its error log, `<sheet>_<dataset>_<YYYY-MM-DD_HH-MM-SS>.error.log`, beside its run log and named like it (see [CSV Output](#csv-output)), with one entry per sheet refused, containing:
- The time the entry was written (UTC)
- The top of the run log: the Excel file, each sheet, the dataset and the data dictionary, each with its name and ID
- Total count of rows with errors
- Detailed error information for each failed row
- Complete row data for debugging

If the error log cannot be written, the details are printed with the error instead. Other errors, such as duplicate headers, are printed and kept in the run log only.

Example entry:
```
[2024-01-15T10:30:45.123456789+00:00] Excel Validation Error Report:
Excel file: samples.xlsx
Excel sheet: Sheet1
Dataset Name: Iberian Peninsula - Atlantic Ocean - 2026
Dataset ID: ce6d5e1b-4367-4c54-a295-46b765e40a5e
Dictionary Name: Analytical results
Dictionary ID: 6bc71c4e-a63b-436b-98c1-b74363fb482e


=============================

Total rows with errors: 1

Row 3: 1 error(s)
Row data: {
  "sample_id": "",
  "collection_date": "invalid-date",
  "latitude": "not-a-number"
}
Errors:
  - Required field missing at row[3]: sample_id
  - Invalid format at row[3]: collection_date must be a valid date
```

### CSV Output
Valid data is saved to a CSV file in the current directory, beside the run log, and uploaded under the same name:
```
<sheet>_<dataset>_<YYYY-MM-DD_HH-MM-SS>.csv
```

What changes least often comes first, so that, sorted by name, the files group by sheet, then by dataset, each group in time order, with each CSV file beside its run log and, if rows failed validation, its error log (`.error.log`).

A run merging `n` sheets has `<words>` in place of `<sheet>`, the whole words every sheet name starts with (left out if there are none), and ends in `_<n>_merged`, so that merges into a dataset group together in time order, however many sheets each merged. Merging "Analytical results (Air)" and "Analytical results (Sediment)" gives:
```
analytical_results_<dataset>_<YYYY-MM-DD_HH-MM-SS>_2_merged.csv
```

## Authentication

The application requires HTTPS for security when using basic authentication. The username and password are used to authenticate with the DKAN API endpoints.

## Error Handling

The application handles various validation scenarios:
- **Type Coercion**: Intelligent conversion of Excel data types to schema-expected types
- **Date Parsing**: Multiple date format recognition and conversion
- **Number Formatting**: Handles various number formats and separators
- **Boolean Recognition**: Recognizes common boolean representations
- **Schema Violations**: Detailed reporting of validation failures

## License

This project is licensed under the MIT License.
