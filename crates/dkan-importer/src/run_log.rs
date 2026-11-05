//! The run log: everything a run prints, every change it made to a cell with its reason, and the
//! questions about the data it could not decide alone, so the data can be checked and corrected
//! at its source. One file per dataset and sheet, rewritten by every run, so it always describes
//! the latest one.

use excel_core::utils::normalize_string;
use excel_core::{Change, Question};
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

/// The run log's lines for `changes`: one per cell, as a sheet can have thousands
pub fn changes_lines(changes: &[Change]) -> Vec<String> {
    if changes.is_empty() {
        return vec!["No cell changed: every cell was taken as written.".to_string()];
    }
    let mut lines = vec![format!(
        "{} cell change(s), each with its reason:",
        changes.len()
    )];
    lines.extend(changes.iter().map(|change| format!("  {change}")));
    lines
}

/// The run log's lines for `questions`, each with the number of cells concerned; none without
/// questions
pub fn questions_lines(questions: &[(Question, usize)]) -> Vec<String> {
    if questions.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "{} question(s) about the data, for the data owners to answer. The cells concerned are listed above.",
        questions.len()
    )];
    lines.extend(
        questions
            .iter()
            .map(|(question, cells)| format!("  [{cells} cell(s)] {question}")),
    );
    lines
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

    /// Lines for the log only
    pub fn notes(&mut self, lines: &[String]) -> io::Result<()> {
        lines.iter().try_for_each(|line| self.note(line))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use excel_core::{SheetCell, SkipReason};

    fn cell() -> SheetCell {
        SheetCell {
            row: 4,
            column: "L".to_string(),
            header: Some("ST03".to_string()),
        }
    }

    #[test]
    fn changes_are_counted_and_listed_one_per_line() {
        let changes = [
            Change::Skipped {
                cell: cell(),
                reason: SkipReason::Blank,
            },
            Change::Skipped {
                cell: cell(),
                reason: SkipReason::Open(Question::NumberInPeakArea),
            },
        ];

        let expected = vec![
            "2 cell change(s), each with its reason:",
            "  row 4, column L \"ST03\": no row: blank cell, no result",
            "  row 4, column L \"ST03\": no row: a number in a Peak Area row",
        ];
        assert_eq!(changes_lines(&changes), expected);
    }

    #[test]
    fn without_changes_the_log_says_every_cell_was_taken_as_written() {
        let expected = vec!["No cell changed: every cell was taken as written."];
        assert_eq!(changes_lines(&[]), expected);
    }

    #[test]
    fn questions_are_listed_once_each_with_their_cell_count() {
        let questions = [
            (Question::NumberInPeakArea, 3),
            (Question::OtherUnits("ng/mL".to_string()), 12),
        ];

        let expected = vec![
            "2 question(s) about the data, for the data owners to answer. The cells concerned are listed above.",
            "  [3 cell(s)] Numbers in Peak Area rows are not imported. Should they be imported as the Concentration, with which detection characterisation, and should a field mark them as peak areas?",
            "  [12 cell(s)] Rows with Concentration units \"ng/mL\" are not imported. How should they be handled?",
        ];
        assert_eq!(questions_lines(&questions), expected);
    }

    #[test]
    fn without_questions_the_log_has_no_questions_section() {
        assert_eq!(questions_lines(&[]), Vec::<String>::new());
    }
}
