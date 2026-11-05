#![allow(clippy::needless_return, non_snake_case)]

// Re-export common dependencies for use by dependent crates
pub use anyhow;
pub use log;
pub use reqwest;
pub use serde;
pub use serde_json;

mod excel_validator;
pub mod utils;
pub mod writers;

// Test utilities - only compiled when testing or with test feature
// #[cfg(test)] alone doesn't work for integration tests (they're external crates)
// The feature flag makes it available to integration tests via dev-dependencies
#[cfg(any(test, feature = "test"))]
pub mod test_utils;

pub use excel_validator::{
    Change, DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, DETECTION_FIELD, ExcelValidator,
    ExcelValidatorBuilder, FieldSchema, IDENTIFICATION_LEVEL, IDENTIFICATION_LEVEL_FIELD,
    LOD_FIELD, LOQ_FIELD, MarkerLimit, Question, SchemaType, SheetCell, SkipReason, UNITS_FIELD,
    Unpivot, parse_values_marker,
};

pub const ERRORS_LOG_FILE: &str = "errors.log";
