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
