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
- **`--sheet-name`** (optional) - Name of the Excel sheet to process (defaults to "Sheet1")
- **`--unpivot`** (optional) - Unpivot the sheet: one row per sample cell. Requires `--headers-column`. See [Unpivoting a wide sheet](#unpivoting-a-wide-sheet)
- **`--headers-column`** (with `--unpivot` only) - The data dictionary field that receives each sample column's header
- **`--values-column`** (with `--unpivot` only) - The data dictionary field that receives each sample cell's value (defaults to "Concentration")
- **`--values-markers`** (with `--unpivot` only) - The text a sample cell may hold instead of a number when the values column is a text field (defaults to `<LOD` `<LOQ` `NS`). See [Markers instead of numbers](#markers-instead-of-numbers)
- **`--marker-limits`** (with `--unpivot` only) - The markers that may be followed by their limit, as `MARKER=FIELD` (defaults to `"<LOD=Limit of detection"` `"<LOQ=Limit of quantification"`). See [A marker followed by its limit](#a-marker-followed-by-its-limit)
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
- `--headers-column` and `--values-column` are only accepted with `--unpivot`, and `--unpivot` needs `--headers-column`. Both must be data dictionary fields that are not columns in the sheet. Otherwise the run stops and lists the fields that can be used.
- The run also stops, before anything is written or uploaded, if the sheet does not start with data dictionary fields, has no sample columns, or has a data dictionary field among the sample columns.
- Before validating, it prints what it will do, for example: `Keeping 9 column(s) A-I (Anal. Method Serial No. … Limit of quantification); unpivoting 74 column(s) J-CE (ST01_Surface … ST41_Bottom) into Sample Tag / Concentration: 518 row(s).`
- Validation errors name the source row. The row data in `errors.log` includes the sample column's header.

#### Markers instead of numbers

Some results are not numbers: `<LOD` (below the limit of detection), `<LOQ` (below the limit of quantification) or `NS` (no sample). The limits themselves are in their own columns, so these markers are kept as written. That needs a values column that is a text field (`string`) in the data dictionary. A number field refuses them as type mismatches.

A text field would accept any text, so the importer checks every non-blank sample cell: it must be a number or one of the `--values-markers`. Anything else, such as `<LDO` or `n.d.`, is a validation error at its source row:

```
Invalid format at row[2]./Concentration: '<LDO' is not a number or one of the markers <LOD, <LOQ, NS
```

- The default markers are `<LOD`, `<LOQ` and `NS`. Giving `--values-markers` replaces them, so list all of them: `--values-markers "<LOD" "<LOQ" "NS" "<MDL"`.
- `ND`, `<LQ` and `NQ` are refused, even when given in `--values-markers`: `ND` maps to `NS`, and `<LQ` and `NQ` map to `<LOQ`, so change them in the source file, for consistency. A sample cell with any of them (any case, also with a limit, as in `<LQ:0.25`) is a validation error saying so: `'ND' is not allowed: ND maps to NS. Change it to NS in the source file, for consistency.`
- Markers match ignoring case and are written uppercase, so `<lod` is uploaded as `<LOD`, also when given in `--values-markers` or `--marker-limits`. Spaces around a cell's text are ignored, so `NS ` is `NS`.
- Numbers are uploaded as Excel stores them (`0.880999866251062`), and blank cells are uploaded empty.
- With a number field the markers are not used, because the field already refuses all text.
- The summary line ends with the rule, for example `... 518 row(s). Concentration takes numbers or <LOD, <LOQ, NS.`

#### A marker followed by its limit

Some sheets write the limit after the marker, as in `<LOQ:0.25`, although the same row already has it in its "Limit of quantification" column. When the number after the marker equals the row's limit, the cell is uploaded as the marker alone (`<LOQ`), the same as in sheets without the number. Spaces around the `:` are ignored, and numbers compare as numbers, so `<LOQ : 0.250` matches a limit of `0.25`.

A different number, or a row whose limit is blank, is a validation error, so no number is dropped that the row does not keep:

```
Invalid format at row[2]./Concentration: '<LOQ:0.3' does not end with the row's Limit of quantification (0.25); write '<LOQ' or '<LOQ:0.25'
Invalid format at row[3]./Concentration: '<LOQ:0.25' has a number, but the row has no Limit of quantification
```

- The default limits are `"<LOD=Limit of detection"` and `"<LOQ=Limit of quantification"`. Giving `--marker-limits` replaces them. `NS`, or any marker without a limit, takes no number.
- A limit for a marker that is not one of the `--values-markers` does not apply. Its field must be a data dictionary field, or the run stops before reading any rows.
- The summary line counts the cells written as the marker alone, for example `... Concentration takes numbers or <LOD, <LOQ, NS. 375 value(s) written as their marker alone, the number after it being the row's limit: <LOD (Limit of detection), <LOQ (Limit of quantification).`

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
- Cells containing Excel error values (`#N/A`, `#DIV/0!`, ...) stop the import with a list of the affected cells
- Empty cells in numeric columns are uploaded as `000000000000.000000` (number) or `0` (integer), so that DKAN creates numeric columns

Example Excel structure:
```
| sample_id | collection_date | latitude | longitude | temperature |
| --------- | --------------- | -------- | --------- | ----------- |
| SAMPLE001 | 2024-01-15      | 45.123   | 12.456    | 18.5        |
| SAMPLE002 | 2024-01-16      | 45.124   | 12.457    | 19.2        |
```

## Workflow

1. **Authentication**: Connects to DKAN with provided credentials
2. **Schema Retrieval**: Fetches the specified data dictionary from DKAN
3. **Schema Conversion**: Converts DKAN data dictionary to JSON Schema
4. **Validation**: Validates each Excel row against the schema
5. **CSV Export**: Exports valid data to a timestamped CSV file
6. **Upload**: Uploads the CSV file to DKAN's custom importer endpoint
7. **Distribution**: Adds the uploaded CSV as a distribution to the specified dataset

## Output

### Console Output
- Authentication status
- Schema conversion progress
- Real-time validation results
- Upload progress and success confirmation

### Error Log File
The application creates an `errors.log` file containing:
- Timestamp of validation run
- Total count of rows with errors
- Detailed error information for each failed row
- Complete row data for debugging

Example error log:
```
Excel Validation Error Report
=============================
Generated at: 2024-01-15 10:30:45

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
Valid data is exported to a timestamped CSV file with the format:
```
{dataset_id}_{data_dictionary_id}_{YYYY-MM-DD_HH-MM-SS}.csv
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
