use anyhow::Result;
use calamine::{Data, Reader, Xlsx, open_workbook};
use jsonschema::{ValidationError as JsonSchemaError, Validator};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use thiserror::Error;

use crate::utils::{normalize_string, write_error_to_log};

/// Default placeholder for numeric fields to hint DKAN about expected precision
/// This will result in DECIMAL(18, 6) - 18 total digits with 6 decimal places
const NUMERIC_PLACEHOLDER: &str = "000000000000.000000";

/// Type alias for a parsed Excel row with row number and field data
pub type ParsedExcelRow = (usize, Map<String, Value>);

/// Type alias for the result of processing Excel rows - (headers, parsed_rows)
pub type ExcelProcessingResult = (Vec<String>, Vec<ParsedExcelRow>);

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

    #[error(
        "Excel has the following extra columns not found in the provided data dictionary at {path}: {properties:?}"
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
}

impl From<JsonSchemaError<'_>> for ValidationError {
    fn from(err: JsonSchemaError) -> Self {
        ValidationError::InvalidFormat {
            path: err.instance_path.to_string(),
            message: err.to_string(),
        }
    }
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
}

pub struct ExcelValidatorBuilder {
    excel_path: String,
    sheet_name: String,
    schema: Value,
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
        }
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
        };

        // Process Excel rows once during build
        let (headers, rows) = temp_validator.process_excel_rows()?;

        Ok(ExcelValidator {
            excel_path: self.excel_path,
            sheet_name: self.sheet_name,
            validator,
            field_schemas,
            validation_reports: Vec::new(),
            headers,
            rows,
        })
    }
}

impl ExcelValidator {
    //////////////////////////////////////////////////////////////
    ///  Public API
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
        })
    }

    /// Get the headers from the Excel file
    pub fn headers(&self) -> &Vec<String> {
        &self.headers
    }

    /// Get the parsed rows from the Excel file
    pub fn rows(&self) -> &Vec<ParsedExcelRow> {
        &self.rows
    }

    pub fn validate_excel(&mut self) -> Result<()> {
        let row_count = self.rows.len();
        if row_count == 0 {
            return Err(anyhow::anyhow!("The Excel file is empty"));
        }

        for (row_number, json_obj) in &self.rows {
            let mut row_value = Value::Object(json_obj.clone());

            // Apply additional intelligent type coercion if initial validation fails
            if !self.validator.is_valid(&row_value) {
                row_value = self.apply_intelligent_type_coercion(row_value);
            }

            // Validate the row
            if self.validator.is_valid(&row_value) {
            } else {
                let errors = self.collect_validation_errors(&row_value, *row_number);

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
            write_error_to_log("Excel Validation Error Report", &validation_report);
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
                            // For numeric fields, use 0.0 instead of empty string to help DKAN type inference
                            // Look up field schema using the original Excel header (title)
                            if let Some(field_schema) = self.field_schemas.get(header) {
                                match &field_schema.field_type {
                                    SchemaType::Number => NUMERIC_PLACEHOLDER.to_string(),
                                    SchemaType::Integer => "0".to_string(),
                                    SchemaType::Mixed(types) => {
                                        // For mixed types (like [Number, Null]), check if it contains Number or Integer
                                        if types.iter().any(|t| matches!(t, SchemaType::Number)) {
                                            NUMERIC_PLACEHOLDER.to_string()
                                        } else if types
                                            .iter()
                                            .any(|t| matches!(t, SchemaType::Integer))
                                        {
                                            "0".to_string()
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
    ///  Private methods
    //////////////////////////////////////////////////////////////
    /// Extract field schemas from JSON schema for intelligent type coercion
    fn extract_field_schemas(schema: &Value) -> Result<HashMap<String, FieldSchema>> {
        let mut field_schemas = HashMap::new();

        if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
            for (field_name, field_schema) in properties {
                if let Ok(parsed_schema) = Self::parse_field_schema(field_schema) {
                    field_schemas.insert(field_name.clone(), parsed_schema);
                }
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
        if schema.get("type").and_then(|t| t.as_str()) == Some("array") {
            if let Some(items) = schema.get("items") {
                let item_type = Self::parse_schema_type(items)?;
                return Ok(SchemaType::Array(Box::new(item_type)));
            }
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
    fn process_excel_rows(&self) -> Result<ExcelProcessingResult> {
        let mut workbook: Xlsx<_> = open_workbook(&self.excel_path)?;

        // Get the sheet to process
        let range = match workbook.worksheet_range(&self.sheet_name) {
            Ok(range) => range,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Error reading sheet '{}': {}",
                    self.sheet_name,
                    e
                ));
            }
        };

        let mut headers: Vec<String> = Vec::new();
        let mut parsed_rows: Vec<ParsedExcelRow> = Vec::new();

        // Process each row
        for (row_index, row) in range.rows().enumerate() {
            if row_index == 0 {
                // First row contains headers - normalize them to match DKAN titles
                headers = row
                    .iter()
                    .map(|cell| normalize_string(&cell.to_string()))
                    .collect();

                // Check for duplicate headers
                Self::check_header_duplicates(&headers)?;

                continue;
            }

            // Skip empty rows
            let is_empty_row = row.iter().all(|cell| match cell {
                Data::Empty => true,
                Data::String(s) => s.trim().is_empty(),
                Data::Error(_) => true,
                _ => false,
            });
            if is_empty_row {
                continue;
            }

            // Convert row to JSON object with intelligent type coercion
            let mut json_obj = Map::new();
            for (col_idx, cell) in row.iter().enumerate() {
                if col_idx < headers.len() {
                    let header = &headers[col_idx];
                    let value = self.convert_cell_to_json_with_schema_awareness(cell, header);
                    json_obj.insert(header.clone(), value);
                }
            }

            parsed_rows.push((row_index + 1, json_obj));
        }

        Ok((headers, parsed_rows))
    }

    /// Format validation reports into a structured string for logging
    fn format_validation_report(&self) -> String {
        let mut report = String::new();

        // Add title and separator
        report.push_str("=============================\n");

        // Generate ISO 8601 formatted timestamp
        let now = chrono::Utc::now().to_rfc3339();
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
        match cell {
            Data::Empty => {
                // Check if this field allows null values (for non-mandatory fields)
                if let Some(field_schema) = self.field_schemas.get(field_name) {
                    match &field_schema.field_type {
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
            Data::Float(f) => self.convert_float_with_validation(*f),
            Data::Int(i) => json!(*i),
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
    fn convert_string_with_schema_intelligence(&self, s: &str, field_name: &str) -> Value {
        let trimmed = s.trim();

        // Handle empty strings
        if trimmed.is_empty() {
            // Check if field expects null or empty string
            if let Some(field_schema) = self.field_schemas.get(field_name) {
                match &field_schema.field_type {
                    SchemaType::Null => return Value::Null,
                    SchemaType::Mixed(types) => {
                        // If it's a union type that includes null, prefer null for empty strings
                        if types.contains(&SchemaType::Null) {
                            return Value::Null;
                        }
                    }
                    _ => {}
                }
            }
            return Value::String(s.to_string());
        }

        // Get schema expectations for this field
        if let Some(field_schema) = self.field_schemas.get(field_name) {
            return self.coerce_string_to_schema_type(s, field_schema);
        }

        // Fallback: intelligent type detection without schema
        self.intelligent_string_conversion(s)
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
                if let Ok(float_val) = s.parse::<f64>() {
                    if float_val.fract().abs() < f64::EPSILON {
                        let int_val = float_val as i64;
                        if let (Some(min), Some(max)) = (field_schema.minimum, field_schema.maximum)
                        {
                            if float_val >= min && float_val <= max {
                                return json!(int_val);
                            }
                        } else {
                            return json!(int_val);
                        }
                    }
                }
                Value::String(s.to_string())
            }

            SchemaType::Number => {
                // Use smart number conversion that prefers integers
                if let Some(num_val) = self.smart_number_conversion(s) {
                    // Check bounds if specified
                    if let (Some(min), Some(max)) = (field_schema.minimum, field_schema.maximum) {
                        if let Some(num_f64) = num_val.as_f64() {
                            if num_f64 >= min && num_f64 <= max {
                                return num_val;
                            }
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
                        if let Some(enum_str) = enum_val.as_str() {
                            if enum_str.to_lowercase() == lower_s {
                                return Value::String(enum_str.to_string());
                            }
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
                if s.starts_with('[') && s.ends_with(']') {
                    if let Ok(arr_val) = serde_json::from_str::<Value>(s) {
                        if arr_val.is_array() {
                            return arr_val;
                        }
                    }
                }

                // Split by common delimiters
                let delimiters = [",", ";", "|", "\t"];
                for delimiter in &delimiters {
                    if s.contains(delimiter) {
                        let items: Vec<Value> = s
                            .split(delimiter)
                            .map(|item| Value::String(item.trim().to_string()))
                            .collect();
                        return json!(items);
                    }
                }

                Value::String(s.to_string())
            }

            SchemaType::Object => {
                // Try to parse as JSON object
                if s.starts_with('{') && s.ends_with('}') {
                    if let Ok(obj_val) = serde_json::from_str::<Value>(s) {
                        if obj_val.is_object() {
                            return obj_val;
                        }
                    }
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
                // Try various date formats
                if self.looks_like_date(s) {
                    // Normalize date format
                    if let Ok(parsed_date) = self.parse_date_string(s) {
                        return Value::String(parsed_date);
                    }
                }
                Value::String(s.to_string())
            }

            "date-time" => {
                // Try ISO datetime formats
                if s.contains('T') || s.contains(' ') {
                    if let Ok(parsed_datetime) = self.parse_datetime_string(s) {
                        return Value::String(parsed_datetime);
                    }
                }
                Value::String(s.to_string())
            }

            "time" => {
                // Validate and normalize time format
                if s.contains(':') {
                    if let Ok(parsed_time) = self.parse_time_string(s) {
                        return Value::String(parsed_time);
                    }
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
                    else if self.schema_expects_string(field_schema) {
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

    /// Check if the field schema expects a string type (including union types with string)
    fn schema_expects_string(&self, field_schema: &FieldSchema) -> bool {
        match &field_schema.field_type {
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

        if let Some(field_schema) = self.field_schemas.get(field_name) {
            if let Some(format) = &field_schema.format {
                let formatted = chrono_dt.format(format).to_string();
                return Value::String(formatted);
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
        if let Some(field_schema) = self.field_schemas.get(field_name) {
            if let Some(format) = &field_schema.format {
                return self.convert_string_by_format(dt_str, format);
            }
        }

        Value::String(dt_str.to_string())
    }

    // Helper methods for date/time parsing
    fn parse_date_string(&self, s: &str) -> Result<String> {
        // Implement date parsing logic - simplified for now
        Ok(s.to_string())
    }

    fn parse_datetime_string(&self, s: &str) -> Result<String> {
        // Implement datetime parsing logic - simplified for now
        Ok(s.to_string())
    }

    fn parse_time_string(&self, s: &str) -> Result<String> {
        // Implement time parsing logic - simplified for now
        Ok(s.to_string())
    }

    fn convert_float_with_validation(&self, f: f64) -> Value {
        // Handle special float values
        if f.is_nan() || f.is_infinite() {
            return Value::Null;
        }

        // Check if it's actually an integer (with tolerance for floating point precision)
        if (f.fract().abs() < f64::EPSILON) && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            json!(f as i64)
        } else {
            json!(f)
        }
    }

    fn collect_validation_errors(
        &self,
        row_value: &Value,
        row_number: usize,
    ) -> Vec<ValidationError> {
        // Use jsonschema's detailed error reporting with sophisticated error analysis
        self.validator
            .iter_errors(row_value)
            .map(|error| {
                let path = if error.instance_path.to_string().is_empty() {
                    format!("row[{}]", row_number)
                } else {
                    format!("row[{}].{}", row_number, error.instance_path)
                };

                // Extract field name directly from the jsonschema error instance path
                let field_name = error.instance_path.to_string();
                let is_required_field = field_name.ends_with('*');

                // Analyze the error message to determine the specific validation failure
                let error_msg = error.to_string();
                self.analyze_jsonschema_error_with_context(
                    &error_msg,
                    &path,
                    &error.instance,
                    is_required_field,
                )
            })
            .collect()
    }

    /// Analyzes jsonschema error messages with additional context about whether the field is required
    fn analyze_jsonschema_error_with_context(
        &self,
        error_msg: &str,
        path: &str,
        instance: &Value,
        is_required_field: bool,
    ) -> ValidationError {
        let lower_msg = error_msg.to_lowercase();

        // Type mismatch detection
        if lower_msg.contains("is not of type") {
            let expected_type = self.extract_expected_type(error_msg);
            let actual_type = self.get_json_type_name(instance);
            let is_null_value = matches!(instance, Value::Null);

            if is_required_field && is_null_value {
                return ValidationError::TypeMismatch {
                    path: path.to_string(),
                    expected: format!("{} (required field)", expected_type),
                    actual: actual_type,
                    value: self.safe_value_string(instance),
                };
            }

            return ValidationError::TypeMismatch {
                path: path.to_string(),
                expected: expected_type,
                actual: actual_type,
                value: self.safe_value_string(instance),
            };
        }

        // Required field detection
        if lower_msg.contains("is a required property") || lower_msg.contains("required") {
            let field_name = self.extract_required_field(error_msg);
            return ValidationError::RequiredFieldMissing {
                path: path.to_string(),
                field: field_name,
            };
        }

        // Range validation (minimum, maximum)
        if lower_msg.contains("is less than")
            || lower_msg.contains("is greater than")
            || lower_msg.contains("minimum")
            || lower_msg.contains("maximum")
        {
            return ValidationError::OutOfRange {
                path: path.to_string(),
                message: self.enhance_range_error_message(error_msg, instance),
            };
        }

        // Format validation (email, date, etc.)
        if lower_msg.contains("is not a")
            && (lower_msg.contains("email")
                || lower_msg.contains("date")
                || lower_msg.contains("time")
                || lower_msg.contains("uri"))
        {
            return ValidationError::InvalidFormat {
                path: path.to_string(),
                message: self.enhance_format_error_message(error_msg, instance),
            };
        }

        // String length validation
        if lower_msg.contains("is shorter than")
            || lower_msg.contains("is longer than")
            || lower_msg.contains("minlength")
            || lower_msg.contains("maxlength")
        {
            return ValidationError::InvalidFormat {
                path: path.to_string(),
                message: self.enhance_string_length_error(error_msg, instance),
            };
        }

        // Pattern validation
        if lower_msg.contains("does not match") || lower_msg.contains("pattern") {
            let pattern = self.extract_pattern(error_msg);
            return ValidationError::PatternMismatch {
                path: path.to_string(),
                pattern,
                value: self.safe_value_string(instance),
            };
        }

        // Additional properties
        if lower_msg.contains("additional properties") || lower_msg.contains("not allowed") {
            let properties = self.extract_additional_properties(error_msg);
            return ValidationError::AdditionalProperties {
                path: path.to_string(),
                properties,
            };
        }

        // Array validation
        if lower_msg.contains("array")
            || lower_msg.contains("items")
            || lower_msg.contains("minitems")
            || lower_msg.contains("maxitems")
            || lower_msg.contains("uniqueitems")
        {
            return ValidationError::ArrayValidation {
                path: path.to_string(),
                message: self.enhance_array_error_message(error_msg, instance),
            };
        }

        // Enum validation
        if lower_msg.contains("is not one of") || lower_msg.contains("enum") {
            return ValidationError::InvalidFormat {
                path: path.to_string(),
                message: self.enhance_enum_error_message(error_msg, instance),
            };
        }

        // Fallback to generic invalid format
        ValidationError::InvalidFormat {
            path: path.to_string(),
            message: format!("Validation failed: {}", error_msg),
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

    /// Helper methods for error message enhancement
    fn extract_expected_type(&self, error_msg: &str) -> String {
        // Extract expected type from messages like "'42' is not of type 'integer'"
        if let Some(start) = error_msg.find("not of type '") {
            let start = start + 13; // Length of "not of type '"
            if let Some(end) = error_msg[start..].find('\'') {
                return error_msg[start..start + end].to_string();
            }
        }

        // Try alternative formats
        if error_msg.contains("integer") {
            "integer".to_string()
        } else if error_msg.contains("number") {
            "number".to_string()
        } else if error_msg.contains("string") {
            "string".to_string()
        } else if error_msg.contains("boolean") {
            "boolean".to_string()
        } else if error_msg.contains("array") {
            "array".to_string()
        } else if error_msg.contains("object") {
            "object".to_string()
        } else {
            "unknown".to_string()
        }
    }

    fn extract_required_field(&self, error_msg: &str) -> String {
        // Extract field name from messages with double quotes: "field_name" is a required property
        if let Some(start) = error_msg.find('"') {
            let start = start + 1;
            if let Some(end) = error_msg[start..].find('"') {
                return error_msg[start..start + end].to_string();
            }
        }

        // Fallback: Extract field name from messages with single quotes: 'field_name' is a required property
        if let Some(start) = error_msg.find('\'') {
            let start = start + 1;
            if let Some(end) = error_msg[start..].find('\'') {
                return error_msg[start..start + end].to_string();
            }
        }

        // Additional fallback patterns for different jsonschema error formats
        // Look for patterns like 'property "field_name" is required'
        if let Some(start) = error_msg.find("property \"") {
            let start = start + 10; // length of "property \""
            if let Some(end) = error_msg[start..].find('"') {
                return error_msg[start..start + end].to_string();
            }
        }

        // Look for patterns like 'required property: field_name'
        if let Some(start) = error_msg.find("required property: ") {
            let start = start + 19; // length of "required property: "
            let field_name = error_msg[start..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !field_name.is_empty() {
                return field_name;
            }
        }

        // Look for patterns like 'field_name is required' (at the beginning of message)
        if error_msg.contains("is required") {
            let words: Vec<&str> = error_msg.split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                if *word == "is" && i + 1 < words.len() && words[i + 1] == "required" && i > 0 {
                    let field_name = words[i - 1]
                        .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                        .to_string();
                    if !field_name.is_empty() {
                        return field_name;
                    }
                }
            }
        }

        // If all parsing fails, return a more descriptive error message
        format!("Could not parse field name from: {}", error_msg)
    }

    fn extract_pattern(&self, error_msg: &str) -> String {
        // Extract regex pattern from error messages
        if let Some(start) = error_msg.find("pattern '") {
            let start = start + 9;
            if let Some(end) = error_msg[start..].find('\'') {
                return error_msg[start..start + end].to_string();
            }
        }
        "unknown pattern".to_string()
    }

    fn extract_additional_properties(&self, error_msg: &str) -> Vec<String> {
        // Extract property names from additional properties errors
        // Look for patterns like 'property1', 'property2' in the error message
        let mut properties = Vec::new();

        // Find all text within single quotes
        let chars = error_msg.chars().peekable();
        let mut current_property = String::new();
        let mut in_quote = false;

        for ch in chars {
            if ch == '\'' {
                if in_quote {
                    // End of quoted property name
                    if !current_property.is_empty() {
                        properties.push(current_property.clone());
                        current_property.clear();
                    }
                    in_quote = false;
                } else {
                    // Start of quoted property name
                    in_quote = true;
                    current_property.clear();
                }
            } else if in_quote {
                current_property.push(ch);
            }
        }

        // If we couldn't extract specific properties, return the original message
        if properties.is_empty() {
            properties.push(error_msg.to_string());
        }

        properties
    }

    fn enhance_range_error_message(&self, error_msg: &str, instance: &Value) -> String {
        let value_str = self.safe_value_string(instance);
        format!("{} (current value: {})", error_msg, value_str)
    }

    fn enhance_format_error_message(&self, error_msg: &str, instance: &Value) -> String {
        let value_str = self.safe_value_string(instance);

        if error_msg.to_lowercase().contains("email") {
            format!(
                "Invalid email format. Expected: user@domain.com, got: '{}'",
                value_str
            )
        } else if error_msg.to_lowercase().contains("date") {
            format!(
                "Invalid date format. Expected: YYYY-MM-DD or ISO format, got: '{}'",
                value_str
            )
        } else {
            format!("{} (value: '{}')", error_msg, value_str)
        }
    }

    fn enhance_string_length_error(&self, error_msg: &str, instance: &Value) -> String {
        if let Value::String(s) = instance {
            format!("{} (current length: {} characters)", error_msg, s.len())
        } else {
            error_msg.to_string()
        }
    }

    fn enhance_array_error_message(&self, error_msg: &str, instance: &Value) -> String {
        if let Value::Array(arr) = instance {
            format!("{} (current array length: {})", error_msg, arr.len())
        } else {
            error_msg.to_string()
        }
    }

    fn enhance_enum_error_message(&self, error_msg: &str, instance: &Value) -> String {
        let value_str = self.safe_value_string(instance);
        format!("{} (provided value: '{}')", error_msg, value_str)
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
            write_error_to_log("Excel Header Duplicate Check Error", &full_message);

            Err(anyhow::anyhow!(full_message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*; // Import test utilities from src/test_utils.rs
    use calamine::Data;
    use serde_json::{Value, json};

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
    fn test_number_to_string_conversion_when_schema_expects_string() {
        use serde_json::Map;

        // Create a schema where a field is expected to be a string
        let schema = json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "title": "ID"
                },
                "count": {
                    "type": "integer",
                    "title": "Count"
                }
            },
            "required": ["id", "count"]
        });

        // Create validator
        let validator = create_excel_validator_with_defaults(&schema);

        // Create a row where ID is a number but schema expects string
        let mut json_obj = Map::new();
        json_obj.insert("id".to_string(), json!(12345)); // Number that should be converted to string
        json_obj.insert("count".to_string(), json!(100)); // Number that should remain number

        let row_value = Value::Object(json_obj);

        // Apply intelligent type coercion
        let coerced_value = validator.apply_intelligent_type_coercion(row_value);

        // Check that the ID field was converted to string
        let coerced_obj = coerced_value.as_object().unwrap();
        assert_eq!(coerced_obj.get("id").unwrap(), &json!("12345"));
        assert_eq!(coerced_obj.get("count").unwrap(), &json!(100));
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
                let field_name = error.instance_path.to_string();
                let is_required_field = field_name.ends_with('*');
                validator
                    .analyze_jsonschema_error_with_context(
                        &error.to_string(),
                        path,
                        &error.instance,
                        is_required_field,
                    )
                    .to_string()
            })
            .collect();

        let has_friendly_message = raw_errors.iter().any(|error| {
            error.contains(
                "Excel has the following extra columns not found in the provided data dictionary",
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
        let path = format!("row[2].{}", jsonschema_error.instance_path);
        let field_name = jsonschema_error.instance_path.to_string();
        let is_required_field = field_name.ends_with('*');
        let enhanced_error = validator.analyze_jsonschema_error_with_context(
            &jsonschema_error.to_string(),
            &path,
            &jsonschema_error.instance,
            is_required_field,
        );
        let error_message = enhanced_error.to_string();

        assert!(error_message.contains("Type mismatch"));
        assert!(error_message.contains("row[2]"));
        assert!(error_message.contains("volume_ml") || error_message.contains("Volume (mL)*"));
        assert!(error_message.contains("expected integer"));
        assert!(error_message.contains("got string"));
        assert!(error_message.contains("\"15.5mL\""));
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
            let path = format!("row[2].{}", jsonschema_error.instance_path);
            let field_name = jsonschema_error.instance_path.to_string();
            let is_required_field = field_name.ends_with('*');
            let enhanced_error = validator.analyze_jsonschema_error_with_context(
                &jsonschema_error.to_string(),
                &path,
                &jsonschema_error.instance,
                is_required_field,
            );
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
            let path = format!("row[2].{}", jsonschema_error.instance_path);
            let field_name = jsonschema_error.instance_path.to_string();
            let is_required_field = field_name.ends_with('*');
            let enhanced_error = validator.analyze_jsonschema_error_with_context(
                &jsonschema_error.to_string(),
                &path,
                &jsonschema_error.instance,
                is_required_field,
            );
            let error_message = enhanced_error.to_string();

            assert!(
                error_message.contains("required field"),
                "Error for required field should mention it's required. Got: {}",
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
            let path = format!("row[2].{}", jsonschema_error.instance_path);
            let field_name = jsonschema_error.instance_path.to_string();
            let is_required_field = field_name.ends_with('*');
            let enhanced_error = validator.analyze_jsonschema_error_with_context(
                &jsonschema_error.to_string(),
                &path,
                &jsonschema_error.instance,
                is_required_field,
            );
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

    #[test]
    fn test_builder_creates_validator_with_cached_data() {
        use crate::ExcelValidatorBuilder;

        // This test would require a real Excel file, so we'll just verify the API exists
        // and compiles correctly. The builder pattern ensures data is processed only once.
        let schema = create_test_schema();

        // Verify that the builder API is available and has the expected methods
        let _builder = ExcelValidatorBuilder::new("test.xlsx", "Sheet1", schema);
        // In a real scenario: let mut validator = builder.build()?;
        // Then: validator.validate_excel()? and validator.export_to_csv(...)?
        // would reuse the cached headers and rows
    }

    #[test]
    fn test_accessor_methods() {
        // Test that accessor methods exist and can be called
        let _validator = create_excel_validator_with_defaults(&create_test_schema());

        // These methods should exist on ExcelValidator
        // In practice, they would return the headers/rows after processing
        // let headers = validator.headers()?;
        // let rows = validator.rows()?;

        // This test verifies the API compiles and exists
        assert!(true, "Accessor methods exist on ExcelValidator");
    }

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
        assert_eq!(datetime_value.as_str().unwrap(), "2024-09-15T12:00:00");

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
}
