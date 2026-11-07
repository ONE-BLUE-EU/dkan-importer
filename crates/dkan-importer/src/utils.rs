use crate::import::SheetNames;
use excel_core::utils::{get_local_datetime_with_format, normalize_string};

/// The files a run writes, named alike so they sort together:
/// `<sheet>_<YYYY-MM-DD_HH-MM-SS>_<dataset>`, lowercase, with the sheet name's spaces as `_`.
/// A run merging `n` sheets has `<words>_<n>_merged` in place of `<sheet>`, `<words>` being the
/// words every sheet name starts with, if any, as in `analytical_results_5_merged`.
pub struct RunFiles {
    /// The CSV file uploaded to DKAN
    pub csv: String,
    /// The run log; see [`crate::run_log`]
    pub log: String,
}

impl RunFiles {
    /// The files of a run starting now
    pub fn now(dataset_id: &str, sheets: &SheetNames) -> Self {
        let timestamp = get_local_datetime_with_format("%Y-%m-%d_%H-%M-%S");
        Self::at(dataset_id, sheets, &timestamp)
    }

    /// The files of a run starting at `timestamp`
    pub fn at(dataset_id: &str, sheets: &SheetNames, timestamp: &str) -> Self {
        let sheets = match sheets.merged() {
            None => normalize_string(sheets.first()),
            Some(count) => {
                let words = shared_leading_words(sheets);
                if words.is_empty() {
                    format!("{count} merged")
                } else {
                    format!("{words} {count} merged")
                }
            }
        };
        let stem = format!("{}_{timestamp}_{dataset_id}", sheets.replace(' ', "_")).to_lowercase();
        RunFiles {
            csv: format!("{stem}.csv"),
            log: format!("{stem}.run.log"),
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
    fn the_run_log_is_named_like_the_csv_file_of_the_same_run() {
        let files = RunFiles::at(
            "1234567890",
            &sheets(&["Test Sheet with Spaces"]),
            "2026-11-05_10-00-00",
        );

        let expected_csv = "test_sheet_with_spaces_2026-11-05_10-00-00_1234567890.csv";
        let expected_log = "test_sheet_with_spaces_2026-11-05_10-00-00_1234567890.run.log";
        assert_eq!(files.csv, expected_csv);
        assert_eq!(files.log, expected_log);
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

        let expected_csv = "analytical_results_3_merged_2026-11-05_10-00-00_ds-1.csv";
        let expected_log = "analytical_results_3_merged_2026-11-05_10-00-00_ds-1.run.log";
        assert_eq!(files.csv, expected_csv);
        assert_eq!(files.log, expected_log);
    }

    #[test]
    fn the_files_of_a_merge_of_sheets_sharing_no_words_are_named_by_the_number_of_sheets() {
        let files = RunFiles::at("ds-1", &sheets(&["Water", "Air"]), "2026-11-05_10-00-00");

        assert_eq!(files.csv, "2_merged_2026-11-05_10-00-00_ds-1.csv");
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
            "analytical_2_merged_2026-11-05_10-00-00_ds-1.csv"
        );
    }
}
