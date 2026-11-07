//! The run log: everything a run prints, every change it made to a cell with its reason, and the
//! questions about the data it could not decide alone, so the data can be checked and corrected
//! at its source. One file per run, named like the run's CSV file (see
//! [`crate::utils::RunFiles`]).

use excel_core::{Change, NgPerMlRow, Question};
use std::fs::File;
use std::io::{self, Write};

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

/// The run log's lines for the ng/mL rows skipped, each with the ng/L row holding its values or
/// the closest one; none without ng/mL rows
pub fn ng_per_ml_rows_lines(rows: &[NgPerMlRow]) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "{} ng/mL row(s) skipped, as only ng/L is imported, each with the ng/L row holding its values:",
        rows.len()
    )];
    lines.extend(rows.iter().map(|row| format!("  {row}")));
    lines
}

/// Writes each line both to the terminal and to the run log file. Never give it a credential.
pub struct RunLog {
    file: File,
}

impl RunLog {
    /// Create the file at `path`
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
    use excel_core::{Closest, Counterpart, Difference, SheetCell, SkipReason};
    use std::num::NonZeroUsize;

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
                reason: SkipReason::NoSample("NS".to_string()),
            },
            Change::Skipped {
                cell: cell(),
                reason: SkipReason::Open {
                    question: Question::NumberInPeakArea,
                    reason: "a number in a Peak Area row".to_string(),
                },
            },
        ];

        let expected = vec![
            "2 cell change(s), each with its reason:",
            "  row 4, column L \"ST03\": no row: \"NS\" means no sample",
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
            (Question::OtherUnits("ug/kg".to_string()), 12),
        ];

        let expected = vec![
            "2 question(s) about the data, for the data owners to answer. The cells concerned are listed above.",
            "  [3 cell(s)] Numbers in Peak Area rows are not imported. Should they be imported as the Concentration, with which detection characterisation, and should a field mark them as peak areas?",
            "  [12 cell(s)] Rows with Concentration units \"ug/kg\" are not imported. How should they be handled?",
        ];
        assert_eq!(questions_lines(&questions), expected);
    }

    #[test]
    fn without_questions_the_log_has_no_questions_section() {
        assert_eq!(questions_lines(&[]), Vec::<String>::new());
    }

    #[test]
    fn ng_per_ml_rows_are_counted_and_listed_whether_their_ng_l_row_is_found_or_not() {
        let rows = [
            NgPerMlRow {
                counterpart: Counterpart::Found(23),
                row: 2,
            },
            NgPerMlRow {
                counterpart: Counterpart::Missing(Some(Closest {
                    first: Difference {
                        cell: cell(),
                        expected: "0.25".to_string(),
                        found: "blank".to_string(),
                    },
                    differences: NonZeroUsize::MIN,
                })),
                row: 3,
            },
            NgPerMlRow {
                counterpart: Counterpart::Missing(None),
                row: 5,
            },
        ];

        let expected = vec![
            "3 ng/mL row(s) skipped, as only ng/L is imported, each with the ng/L row holding its values:",
            "  row 2: skipped: in ng/mL; its values are in the ng/L row 23, which is imported",
            "  row 3: skipped: in ng/mL, but no ng/L row has its values (x1000 in the limits and the sample columns); the closest, row 4, differs in 1 cell(s), first at row 4, column L \"ST03\": 0.25 expected, blank found",
            "  row 5: skipped: in ng/mL, but the sheet has no ng/L row to hold its values",
        ];
        assert_eq!(ng_per_ml_rows_lines(&rows), expected);
    }

    #[test]
    fn without_ng_per_ml_rows_the_log_has_no_ng_per_ml_section() {
        assert_eq!(ng_per_ml_rows_lines(&[]), Vec::<String>::new());
    }
}
