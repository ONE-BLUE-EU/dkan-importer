use anyhow::Result;
use calamine::{open_workbook, Data, Reader, Xlsx};
use jsonschema::{ValidationError as JsonSchemaError, Validator};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use thiserror::Error;

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

    #[error("Excel has the following extra columns not found in the provided data dictionary at {path}: {properties:?}")]
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
    pub validator: Validator,
    field_schemas: HashMap<String, FieldSchema>,
    error_log_path: String,
    pub validation_reports: Vec<ValidationReport>,
}

impl ExcelValidator {
    pub fn new(schema: &Value, error_log_path: &str) -> Result<Self> {
        // Create validator from schema
        let validator = jsonschema::validator_for(schema)
            .map_err(|e| anyhow::anyhow!("Invalid JSON schema: {}", e))?;

        // Extract field schemas for intelligent type coercion
        let field_schemas = Self::extract_field_schemas(schema)?;

        Ok(ExcelValidator {
            validator,
            field_schemas,
            error_log_path: error_log_path.to_string(),
            validation_reports: Vec::new(),
        })
    }

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

    /// Normalize Excel header to match DKAN dictionary titles
    /// Replaces newlines and control characters with spaces (but keeps asterisks and full text)
    pub fn normalize_excel_header(raw_header: String) -> String {
        raw_header
            .chars() // Process character by character
            .map(|c| {
                if c.is_control() {
                    ' ' // Replace control characters (newlines, tabs, etc.) with spaces
                } else {
                    c // Keep all other characters including asterisks
                }
            })
            .collect::<String>()
            .split_whitespace() // Split on whitespace to normalize multiple spaces
            .collect::<Vec<&str>>()
            .join(" ") // Join back with single spaces
            .trim() // Remove leading/trailing whitespace
            .to_string()
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
    fn process_excel_rows(
        &self,
        excel_path: &str,
        sheet_name: Option<&str>,
    ) -> Result<ExcelProcessingResult> {
        let mut workbook: Xlsx<_> = open_workbook(excel_path)?;

        // Get the sheet to process
        let sheet_name = sheet_name.unwrap_or("Sheet1");
        let range = match workbook.worksheet_range(sheet_name) {
            Ok(range) => range,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Error reading sheet '{}': {}",
                    sheet_name,
                    e
                ));
            }
        };

        let mut headers: Vec<String> = Vec::new();
        let mut parsed_rows: Vec<ParsedExcelRow> = Vec::new();

        // Process each row
        for (row_idx, row) in range.rows().enumerate() {
            if row_idx == 0 {
                // First row contains headers - normalize them to match DKAN titles
                headers = row
                    .iter()
                    .map(|cell| Self::normalize_excel_header(cell.to_string()))
                    .collect();
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

            parsed_rows.push((row_idx + 1, json_obj));
        }

        Ok((headers, parsed_rows))
    }

    pub fn validate_excel(&mut self, excel_path: &str, sheet_name: Option<&str>) -> Result<()> {
        let (_headers, parsed_rows) = self.process_excel_rows(excel_path, sheet_name)?;
        let row_count = parsed_rows.len();
        if row_count == 0 {
            return Err(anyhow::anyhow!("The Excel file is empty"));
        }

        for (row_number, json_obj) in parsed_rows {
            // Comma check
            for (key, value) in &json_obj {
                if value.is_string() && value.as_str().unwrap().contains(',') {
                    // Store for later reporting
                    self.validation_reports.push(ValidationReport {
                        row_number,
                        errors: vec![ValidationError::InvalidFormat {
                            path: key.to_string(),
                            message: "The value contains a comma".to_string(),
                        }],
                        row_data: json_obj.clone().into(),
                    });
                    continue;
                }
            }

            let mut row_value = Value::Object(json_obj);

            // Apply additional intelligent type coercion if initial validation fails
            if !self.validator.is_valid(&row_value) {
                row_value = self.apply_intelligent_type_coercion(row_value);
                println!(
                    "Value that was coerced: {:?}",
                    serde_json::to_string_pretty(&row_value)
                );
            }

            // Validate the row
            if self.validator.is_valid(&row_value) {
            } else {
                let errors = self.collect_validation_errors(&row_value, row_number);

                // Log to stdout
                for error in &errors {
                    eprintln!("  - {}", error);
                }

                // Store for later reporting
                self.validation_reports.push(ValidationReport {
                    row_number,
                    errors,
                    row_data: row_value,
                });
            }
        }

        self.write_error_log()?;

        Ok(())
    }

    /// Export Excel data to CSV with schema-aware parsing
    pub fn export_to_csv(
        &self,
        excel_path: &str,
        sheet_name: Option<&str>,
        csv_path: &str,
    ) -> Result<()> {
        let (headers, parsed_rows) = self.process_excel_rows(excel_path, sheet_name)?;

        // Configure CSV writer to always quote fields
        let mut wtr = csv::WriterBuilder::new()
            .quote_style(csv::QuoteStyle::Never)
            .from_path(csv_path)?;

        // Write headers - no manual escaping needed, csv writer handles it
        wtr.write_record(&headers)?;

        // Write data rows
        for (_row_number, json_obj) in parsed_rows {
            // Convert parsed JSON values back to CSV record
            let mut csv_record: Vec<String> = Vec::new();
            for header in &headers {
                if let Some(parsed_value) = json_obj.get(header) {
                    // Convert the JSON value to a string for CSV
                    let csv_value = match parsed_value {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => String::new(),
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

    /// Convert cell to JSON with schema awareness for intelligent type coercion
    pub fn convert_cell_to_json_with_schema_awareness(
        &self,
        cell: &Data,
        field_name: &str,
    ) -> Value {
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
                    // Only apply additional coercion to string values that might need conversion
                    if let Some(string_val) = field_value.as_str() {
                        let coerced = self.coerce_string_to_schema_type(string_val, field_schema);
                        if coerced != *field_value {
                            *field_value = coerced;
                        }
                    }
                }
            }
        }
        row_value
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
    pub fn convert_datetime_with_schema_intelligence(
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
    pub fn convert_datetime_string_with_schema_intelligence(
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

    fn write_error_log(&self) -> Result<()> {
        // Only create the error file if we have errors to log
        if self.validation_reports.is_empty() {
            return Ok(()); // No errors, no file needed
        }

        // Create error log file only when we have errors
        let error_file = File::create(&self.error_log_path)?;
        let mut error_log = BufWriter::new(error_file);

        writeln!(error_log, "Excel Validation Error Report")?;
        writeln!(error_log, "=============================")?;

        // Generate ISO 8601 formatted timestamp
        let now = chrono::Utc::now().to_rfc3339();
        writeln!(error_log, "Generated at: {}", now)?;
        writeln!(error_log)?;

        writeln!(
            error_log,
            "Total rows with errors: {}",
            self.validation_reports.len()
        )?;
        writeln!(error_log)?;

        for report in &self.validation_reports {
            writeln!(
                error_log,
                "Row {}: {} error(s)",
                report.row_number,
                report.errors.len()
            )?;
            writeln!(
                error_log,
                "Row data: {}",
                serde_json::to_string_pretty(&report.row_data)?
            )?;
            writeln!(error_log, "Errors:")?;

            for error in &report.errors {
                writeln!(error_log, "  - {}", error)?;
            }
            writeln!(error_log)?;
        }

        error_log.flush()?;
        Ok(())
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

                // Analyze the error message to determine the specific validation failure
                let error_msg = error.to_string();
                self.analyze_jsonschema_error(&error_msg, &path, &error.instance)
            })
            .collect()
    }

    /// Analyzes jsonschema error messages and converts them to appropriate ValidationError types
    pub fn analyze_jsonschema_error(
        &self,
        error_msg: &str,
        path: &str,
        instance: &Value,
    ) -> ValidationError {
        let lower_msg = error_msg.to_lowercase();

        // Type mismatch detection
        if lower_msg.contains("is not of type") {
            let expected_type = self.extract_expected_type(error_msg);
            let actual_type = self.get_json_type_name(instance);

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
        // Extract field name from messages like "'email' is a required property"
        if let Some(start) = error_msg.find('\'') {
            let start = start + 1;
            if let Some(end) = error_msg[start..].find('\'') {
                return error_msg[start..start + end].to_string();
            }
        }
        "unknown field".to_string()
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
        let mut chars = error_msg.chars().peekable();
        let mut current_property = String::new();
        let mut in_quote = false;

        while let Some(ch) = chars.next() {
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

    pub fn looks_like_date(&self, s: &str) -> bool {
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

    pub fn excel_datetime_to_chrono(dt: &calamine::ExcelDateTime) -> chrono::NaiveDateTime {
        use chrono::{Duration, NaiveDate};
        let excel_base = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
        let value = dt.as_f64();
        let days = value as i64;
        let seconds = ((value - days as f64) * 86400.0).round() as i64;
        excel_base.and_hms_opt(0, 0, 0).unwrap() + Duration::days(days) + Duration::seconds(seconds)
    }
}
