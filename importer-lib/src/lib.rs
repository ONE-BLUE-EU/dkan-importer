#![allow(clippy::needless_return, non_snake_case)]

mod excel_validator;
pub mod utils;

// Test utilities - only compiled when testing or with test feature
// #[cfg(test)] alone doesn't work for integration tests (they're external crates)
// The feature flag makes it available to integration tests via dev-dependencies
#[cfg(any(test, feature = "test"))]
pub mod test_utils;

pub use excel_validator::{ExcelValidator, ExcelValidatorBuilder};

pub const ERRORS_LOG_FILE: &str = "errors.log";
