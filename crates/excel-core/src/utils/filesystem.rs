use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::ERRORS_LOG_FILE;
use crate::utils::get_utc_iso_datetime;

/// Centralized function to write error messages to the errors log file
///
/// Returns an error if the log cannot be written, so callers never point the user at a log that
/// does not contain the details.
///
/// # Arguments
/// * `error_type` - A description of the error type/category (e.g., "Data Dictionary Duplicate Check Error")
/// * `error_message` - The actual error message content
pub fn write_error_to_log(error_type: &str, error_message: &str) -> std::io::Result<()> {
    append_error_to_log(Path::new(ERRORS_LOG_FILE), error_type, error_message)
}

fn append_error_to_log(path: &Path, error_type: &str, error_message: &str) -> std::io::Result<()> {
    let timestamp = get_utc_iso_datetime();
    let log_entry = format!("\n[{}] {}:\n{}\n", timestamp, error_type, error_message);

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", log_entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwritable_log_is_reported_as_an_error() {
        let path = std::env::temp_dir()
            .join("excel-core-no-such-dir")
            .join("errors.log");
        assert!(!path.parent().unwrap().exists());

        assert!(append_error_to_log(&path, "Type", "message").is_err());
    }

    #[test]
    fn writable_log_receives_the_message() {
        let path = std::env::temp_dir().join(format!("excel-core-log-{}.log", std::process::id()));

        append_error_to_log(&path, "Some Error Type", "the details").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            content.contains("Some Error Type:\nthe details"),
            "{content}"
        );
    }
}
