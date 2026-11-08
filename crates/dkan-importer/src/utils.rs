use crate::import::SheetNames;
use excel_core::utils::{get_local_datetime_with_format, normalize_string};

/// The files a run writes (the CSV file, the run log and the error log), named alike so they sort
/// together:
/// `<sheet>_<dataset>_<YYYY-MM-DD_HH-MM-SS>`, lowercase, with the sheet name's spaces as `_`.
/// What changes least often comes first, so that sorted by name the files group by sheet, then
/// by dataset, each group in time order. A run merging `n` sheets has `<words>` in place of
/// `<sheet>`, the words every sheet name starts with, if any, and ends in `_<n>_merged`, as in
/// `analytical_results_<dataset>_<YYYY-MM-DD_HH-MM-SS>_5_merged`.
pub struct RunFiles {
    /// The CSV file uploaded to DKAN
    pub csv: String,
    /// The run log; see [`crate::run_log`]
    pub log: String,
    /// The error log, written only when rows fail validation; see [`crate::error_log`]
    pub errors: String,
}

impl RunFiles {
    /// The files of a run starting now
    pub fn now(dataset_id: &str, sheets: &SheetNames) -> Self {
        let timestamp = get_local_datetime_with_format("%Y-%m-%d_%H-%M-%S");
        Self::at(dataset_id, sheets, &timestamp)
    }

    /// The files of a run starting at `timestamp`
    pub fn at(dataset_id: &str, sheets: &SheetNames, timestamp: &str) -> Self {
        let (sheet, merged) = match sheets.merged() {
            None => (normalize_string(sheets.first()), String::new()),
            Some(count) => (shared_leading_words(sheets), format!("_{count}_merged")),
        };
        let start = if sheet.is_empty() {
            String::new()
        } else {
            format!("{}_", sheet.replace(' ', "_"))
        };
        let stem = format!("{start}{dataset_id}_{timestamp}{merged}").to_lowercase();
        RunFiles {
            csv: format!("{stem}.csv"),
            log: format!("{stem}.run.txt"),
            errors: format!("{stem}.error.txt"),
        }
    }
}

/// The words every name of `sheets` starts with, joined by single spaces: "Analytical results"
/// for "Analytical results (Air)" and "Analytical results (Sediment)"
fn shared_leading_words(sheets: &SheetNames) -> String {
    let words = |name: &str| {
        normalize_string(name)
            .split(' ')
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let mut shared = words(sheets.first());
    for name in sheets.iter() {
        let name = words(name);
        let same = shared
            .iter()
            .zip(&name)
            .take_while(|(shared, word)| shared == word)
            .count();
        shared.truncate(same);
    }
    shared.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheets(names: &[&str]) -> SheetNames {
        SheetNames::new(names.iter().map(|name| name.to_string()).collect()).unwrap()
    }

    #[test]
    fn the_run_log_and_the_error_log_are_named_like_the_csv_file_of_the_same_run() {
        let files = RunFiles::at(
            "1234567890",
            &sheets(&["Test Sheet with Spaces"]),
            "2026-11-05_10-00-00",
        );

        let expected_csv = "test_sheet_with_spaces_1234567890_2026-11-05_10-00-00.csv";
        let expected_log = "test_sheet_with_spaces_1234567890_2026-11-05_10-00-00.run.txt";
        let expected_errors = "test_sheet_with_spaces_1234567890_2026-11-05_10-00-00.error.txt";
        assert_eq!(files.csv, expected_csv);
        assert_eq!(files.log, expected_log);
        assert_eq!(files.errors, expected_errors);
    }

    /// Named by the words the sheet names share, and how many sheets are merged
    #[test]
    fn the_files_of_a_merge_are_named_by_the_shared_words_and_the_number_of_sheets() {
        let files = RunFiles::at(
            "ds-1",
            &sheets(&[
                "Analytical results (column wate",
                "Analytical results (MNPLs water",
                "Analytical  results (Air)",
            ]),
            "2026-11-05_10-00-00",
        );

        let expected_csv = "analytical_results_ds-1_2026-11-05_10-00-00_3_merged.csv";
        let expected_log = "analytical_results_ds-1_2026-11-05_10-00-00_3_merged.run.txt";
        let expected_errors = "analytical_results_ds-1_2026-11-05_10-00-00_3_merged.error.txt";
        assert_eq!(files.csv, expected_csv);
        assert_eq!(files.log, expected_log);
        assert_eq!(files.errors, expected_errors);
    }

    #[test]
    fn the_files_of_a_merge_of_sheets_sharing_no_words_are_named_by_the_number_of_sheets() {
        let files = RunFiles::at("ds-1", &sheets(&["Water", "Air"]), "2026-11-05_10-00-00");

        assert_eq!(files.csv, "ds-1_2026-11-05_10-00-00_2_merged.csv");
    }

    /// Sorted by name, the files group by sheet, then by dataset, each group in time order,
    /// merged or not
    #[test]
    fn sorted_by_name_the_files_group_by_sheet_then_dataset_then_time() {
        let air = sheets(&["Analytical results (Air)"]);
        let merged = sheets(&["Analytical results (Air)", "Analytical results (Sediment)"]);
        let runs = [
            ("ds-2", &air, "2026-11-01_09-00-00"),
            ("ds-1", &merged, "2026-11-03_09-00-00"),
            ("ds-1", &air, "2026-11-02_09-00-00"),
            ("ds-2", &merged, "2026-11-01_08-00-00"),
            ("ds-1", &air, "2026-11-01_09-00-00"),
            ("ds-1", &merged, "2026-11-01_10-00-00"),
        ];
        let mut names: Vec<String> = runs
            .iter()
            .flat_map(|(dataset, sheets, timestamp)| {
                let files = RunFiles::at(dataset, sheets, timestamp);
                [files.csv, files.log]
            })
            .collect();

        names.sort();

        let expected = vec![
            "analytical_results_(air)_ds-1_2026-11-01_09-00-00.csv",
            "analytical_results_(air)_ds-1_2026-11-01_09-00-00.run.txt",
            "analytical_results_(air)_ds-1_2026-11-02_09-00-00.csv",
            "analytical_results_(air)_ds-1_2026-11-02_09-00-00.run.txt",
            "analytical_results_(air)_ds-2_2026-11-01_09-00-00.csv",
            "analytical_results_(air)_ds-2_2026-11-01_09-00-00.run.txt",
            "analytical_results_ds-1_2026-11-01_10-00-00_2_merged.csv",
            "analytical_results_ds-1_2026-11-01_10-00-00_2_merged.run.txt",
            "analytical_results_ds-1_2026-11-03_09-00-00_2_merged.csv",
            "analytical_results_ds-1_2026-11-03_09-00-00_2_merged.run.txt",
            "analytical_results_ds-2_2026-11-01_08-00-00_2_merged.csv",
            "analytical_results_ds-2_2026-11-01_08-00-00_2_merged.run.txt",
        ];
        assert_eq!(names, expected);
    }

    /// Whole words only: "Analytical results" and "Analytical resultsX" share "Analytical"
    #[test]
    fn the_shared_words_are_whole_words() {
        let files = RunFiles::at(
            "ds-1",
            &sheets(&["Analytical results", "Analytical resultsX"]),
            "2026-11-05_10-00-00",
        );

        assert_eq!(
            files.csv,
            "analytical_ds-1_2026-11-05_10-00-00_2_merged.csv"
        );
    }
}
