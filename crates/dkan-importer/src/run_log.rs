//! The run log: everything a run prints, every change it made to a cell with its reason, and the
//! questions about the data it could not decide alone, so the data can be checked and corrected
//! at its source. One file per run, named like the run's CSV file (see
//! [`crate::utils::RunFiles`]).

use excel_core::{NgPerMlRow, Question};
use std::fs::File;
use std::io::{self, Write};

/// The run log's lines for `changes`, each its line as written by
/// [`excel_core::ExcelValidator::change_lines`]: one per cell, as a sheet can have thousands
pub fn changes_lines(changes: &[String]) -> Vec<String> {
    if changes.is_empty() {
        return vec!["Every cell was imported as written.".to_string()];
    }
    let mut lines = vec![format!(
        "{} cell(s) not imported exactly as written:",
        changes.len()
    )];
    lines.extend(changes.iter().map(|change| format!("  {change}")));
    lines
}

/// The questions to the data owners, numbered as the cells refer to them ("See Q1."), each with
/// the number of cells concerned; none without questions
pub fn questions_lines(questions: &[(Question, usize)]) -> Vec<String> {
    if questions.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["Questions to data owners:".to_string()];
    lines.extend(
        questions
            .iter()
            .enumerate()
            .map(|(index, (question, cells))| {
                let cells = match cells {
                    1 => "1 cell".to_string(),
                    cells => format!("{cells} cells"),
                };
                format!("Q{} ({cells}): {question}", index.saturating_add(1))
            }),
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

/// Writes each line both to the terminal and to the run log. Never give it a credential.
///
/// `L` is the log, `O` and `E` the standard output and error: files and the terminal for a run,
/// buffers for a test.
pub struct RunLog<L: Write, O: Write, E: Write> {
    log: L,
    out: O,
    err: E,
}

impl RunLog<File, io::Stdout, io::Stderr> {
    /// Create the log file at `path`, printing to the terminal
    pub fn create(path: &str) -> io::Result<Self> {
        Ok(RunLog::new(File::create(path)?, io::stdout(), io::stderr()))
    }
}

impl<L: Write, O: Write, E: Write> RunLog<L, O, E> {
    pub fn new(log: L, out: O, err: E) -> Self {
        RunLog { log, out, err }
    }

    /// The log, the standard output and the standard error, as written
    pub fn into_parts(self) -> (L, O, E) {
        (self.log, self.out, self.err)
    }

    /// A line for the log only, such as what the run was given
    pub fn note(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.log, "{line}")
    }

    /// Lines for the log only
    pub fn notes(&mut self, lines: &[String]) -> io::Result<()> {
        lines.iter().try_for_each(|line| self.note(line))
    }

    /// A line printed to the standard output and logged
    pub fn info(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.out, "{line}")?;
        self.note(line)
    }

    /// A line printed to the standard error and logged
    pub fn error(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.err, "{line}")?;
        self.note(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use excel_core::{CellValue, Closest, Counterpart, Difference, SheetCell};
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
            "row 4, column L \"ST03\": Cell value: \"NS\". Not imported, because no sample was taken.".to_string(),
            "row 5, column L \"ST03\": Cell value: 1234. Not imported, because a number in a Peak Area row has no meaning yet. See Q1.".to_string(),
        ];

        let expected = vec![
            "2 cell(s) not imported exactly as written:",
            "  row 4, column L \"ST03\": Cell value: \"NS\". Not imported, because no sample was taken.",
            "  row 5, column L \"ST03\": Cell value: 1234. Not imported, because a number in a Peak Area row has no meaning yet. See Q1.",
        ];
        assert_eq!(changes_lines(&changes), expected);
    }

    #[test]
    fn without_changes_the_log_says_every_cell_was_imported_as_written() {
        let expected = vec!["Every cell was imported as written."];
        assert_eq!(changes_lines(&[]), expected);
    }

    /// Numbered as the cells refer to them, "See Q1."
    #[test]
    fn questions_are_numbered_each_with_its_cell_count() {
        let questions = [
            (Question::NumberInPeakArea, 1),
            (Question::OtherUnits("ug/kg".to_string()), 12),
        ];

        let expected = vec![
            "Questions to data owners:",
            "Q1 (1 cell): Numbers in Peak Area rows are not imported. Should they be imported as the Concentration, with which detection characterisation, and should a field mark them as peak areas?",
            "Q2 (12 cells): Rows with Concentration units \"ug/kg\" are not imported. How should they be handled?",
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
                        found: CellValue::Blank,
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
            "  row 3: skipped: in ng/mL, but no ng/L row has its values (x1000 in the limits and the sample columns); the closest, row 4, differs in 1 cell(s), first at row 4, column L \"ST03\": Cell value: blank; expected 0.25",
            "  row 5: skipped: in ng/mL, but the sheet has no ng/L row to hold its values",
        ];
        assert_eq!(ng_per_ml_rows_lines(&rows), expected);
    }

    #[test]
    fn without_ng_per_ml_rows_the_log_has_no_ng_per_ml_section() {
        assert_eq!(ng_per_ml_rows_lines(&[]), Vec::<String>::new());
    }
}
