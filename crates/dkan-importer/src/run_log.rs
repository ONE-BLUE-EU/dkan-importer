//! The run log: everything a run prints, and every change it made to a cell with its reason, so
//! the data can be checked and corrected at its source. One file per dataset and sheet, rewritten
//! by every run, so it always describes the latest one.

use excel_core::Change;
use excel_core::utils::normalize_string;
use std::fs::File;
use std::io::{self, Write};

/// `<dataset>_<sheet>.run.log`, with the sheet name lowercase and its spaces as `_`, as in the
/// CSV file name
pub fn run_log_filename(dataset_id: &str, sheet_name: &str) -> String {
    let sheet_name = normalize_string(sheet_name)
        .replace(' ', "_")
        .to_lowercase();
    format!("{dataset_id}_{sheet_name}.run.log")
}

/// Writes each line both to the terminal and to the run log file. Never give it a credential.
pub struct RunLog {
    file: File,
}

impl RunLog {
    /// Create the file at `path`, replacing the log of an earlier run
    pub fn create(path: &str) -> io::Result<Self> {
        Ok(RunLog {
            file: File::create(path)?,
        })
    }

    /// A line for the log only, such as what the run was given
    pub fn note(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.file, "{line}")
    }

    /// A line printed to the standard output and logged
    pub fn info(&mut self, line: &str) -> io::Result<()> {
        println!("{line}");
        self.note(line)
    }

    /// A line printed to the standard error and logged
    pub fn error(&mut self, line: &str) -> io::Result<()> {
        eprintln!("{line}");
        self.note(line)
    }

    /// Every change made to a cell, one per line, for the log only: a sheet can have thousands
    pub fn changes(&mut self, changes: &[Change]) -> io::Result<()> {
        if changes.is_empty() {
            return self.note("No cell changed: every cell was taken as written.");
        }
        self.note(&format!(
            "{} cell change(s), each with its reason:",
            changes.len()
        ))?;
        for change in changes {
            self.note(&format!("  {change}"))?;
        }
        Ok(())
    }
}
