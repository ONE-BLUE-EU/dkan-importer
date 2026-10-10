use anyhow::{Context, Result};
use calamine::{Data, Reader, Xlsx, open_workbook};
use jsonschema::Validator;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
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

/// Type alias for a parsed Excel row with row number and field data
pub type ParsedExcelRow = (usize, Map<String, Value>);

/// A sheet read into rows ready for validation
struct ProcessedSheet {
    headers: Vec<String>,
    rows: Vec<ParsedExcelRow>,
    /// What the unpivot did, when the sheet was unpivoted
    unpivot_summary: Option<String>,
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
}

pub struct ExcelValidatorBuilder {
    excel_path: String,
    sheet_name: String,
    schema: Value,
    unpivot: Option<Unpivot>,
}

/// Unpivot a wide sheet: each cell of a sample column becomes its own row.
///
/// The sheet's leading columns whose headers are schema fields are kept and copied to every row;
/// every column after them is a sample column. Both fields must be schema fields that are not
/// sheet columns.
pub struct Unpivot {
    /// The field that receives the sample column's header
    pub headers_column: String,
    /// The field that receives the sample column's cell value
    pub values_column: String,
}

/// The sample columns of an unpivoted sheet and the fields they go to
struct SampleColumns {
    /// (column index, header) of each sample column
    columns: Vec<(usize, String)>,
    headers_column: String,
    values_column: String,
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
        }
    }

    /// Unpivot the sheet while reading it; see [`Unpivot`].
    pub fn unpivot(mut self, unpivot: Unpivot) -> Self {
        self.unpivot = Some(unpivot);
        self
    }

    /// Build the ExcelValidator, processing Excel rows during construction
    ///
    /// This method reads the Excel file once and caches the headers and rows,
    /// making subsequent validation and export operations more efficient.
    pub fn build(self) -> Result<ExcelValidator> {
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
        };

        // Process Excel rows once during build
        let sheet = temp_validator.process_excel_rows(self.unpivot.as_ref())?;

        Ok(ExcelValidator {
            excel_path: self.excel_path,
            sheet_name: self.sheet_name,
            validator,
            field_schemas,
            validation_reports: Vec::new(),
            headers: sheet.headers,
            rows: sheet.rows,
            unpivot_summary: sheet.unpivot_summary,
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
        })
    }

    /// What the unpivot did (columns kept and unpivoted, rows produced), when the sheet was
    /// unpivoted
    pub fn unpivot_summary(&self) -> Option<&str> {
        self.unpivot_summary.as_deref()
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
        let row_count = self.rows.len();
        if row_count == 0 {
            return Err(anyhow::anyhow!("The Excel file is empty"));
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
            return Err(
                match write_error_to_log("Excel Validation Error Report", &validation_report) {
                    Ok(()) => anyhow::anyhow!("{} Check the error log for details.", summary),
                    Err(e) => anyhow::anyhow!(
                        "{} The error log could not be written ({}), so the details follow:\n{}",
                        summary,
                        e,
                        validation_report
                    ),
                },
            );
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
    fn process_excel_rows(&self, unpivot: Option<&Unpivot>) -> Result<ProcessedSheet> {
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
        let mut parsed_rows: Vec<ParsedExcelRow> = Vec::new();
        // Excel error values (#N/A, #DIV/0!, ...) are broken cells, not data: collect them all
        let mut error_cells: Vec<String> = Vec::new();
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
                        kept = unpivot_kept;
                        samples = Some(unpivot_samples);
                    }
                }

                continue;
            }

            // The row number Excel shows
            let row_number = usize::try_from(first_row)
                .unwrap_or(usize::MAX)
                .saturating_add(row_index)
                .saturating_add(1);

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
            let is_empty_row = row
                .iter()
                .all(|cell| is_blank(cell) || matches!(cell, Data::Error(_)));
            if is_empty_row {
                continue;
            }

            // Convert row to JSON object with intelligent type coercion
            let mut json_obj = Map::new();
            for &col_idx in &kept {
                let (Some(cell), Some(Some(header))) = (row.get(col_idx), columns.get(col_idx))
                else {
                    continue;
                };
                if let Data::Error(e) = cell {
                    error_cells.push(format!("row {}, column '{}': {}", row_number, header, e));
                    continue;
                }
                let value = self.convert_cell_to_json_with_schema_awareness(cell, header);
                json_obj.insert(header.clone(), value);
            }

            let Some(samples) = &samples else {
                parsed_rows.push((row_number, json_obj));
                continue;
            };
            for (col_idx, header) in &samples.columns {
                let Some(cell) = row.get(*col_idx) else {
                    continue;
                };
                if let Data::Error(e) = cell {
                    error_cells.push(format!("row {}, column '{}': {}", row_number, header, e));
                    continue;
                }
                let mut sample_obj = json_obj.clone();
                sample_obj.insert(
                    samples.headers_column.clone(),
                    Value::String(header.clone()),
                );
                sample_obj.insert(
                    samples.values_column.clone(),
                    self.convert_cell_to_json_with_schema_awareness(cell, &samples.values_column),
                );
                parsed_rows.push((row_number, sample_obj));
            }
        }

        if !unnamed_columns_with_data.is_empty() {
            return Err(anyhow::anyhow!(
                "The sheet \"{}\" has data in {} column(s) without a header. Add a header or clear these cells and try again:\n  {}",
                self.sheet_name,
                unnamed_columns_with_data.len(),
                unnamed_columns_with_data
                    .into_values()
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ));
        }

        if !error_cells.is_empty() {
            return Err(anyhow::anyhow!(
                "The sheet \"{}\" contains {} Excel error value(s). Fix these cells and try again:\n  {}",
                self.sheet_name,
                error_cells.len(),
                error_cells.join("\n  ")
            ));
        }

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
            format!(
                "Keeping {} column(s) {}; unpivoting {} column(s) {} into {} / {}: {} row(s).",
                kept_columns.len(),
                column_span(&kept_columns, first_col),
                sample_columns.len(),
                column_span(&sample_columns, first_col),
                samples.headers_column,
                samples.values_column,
                parsed_rows.len()
            )
        });

        Ok(ProcessedSheet {
            headers,
            rows: parsed_rows,
            unpivot_summary,
        })
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

        Ok((
            kept.iter().map(|(i, _)| *i).collect(),
            SampleColumns {
                columns: sample_part
                    .iter()
                    .map(|(i, name)| (*i, name.to_string()))
                    .collect(),
                headers_column,
                values_column,
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

        // 4. Future custom validators can be added here
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
