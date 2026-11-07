use excel_core::utils::{get_local_datetime_with_format, normalize_string};

/// The files a run writes, named alike so they sort together:
/// `<sheet>_<YYYY-MM-DD_HH-MM-SS>_<dataset>`, lowercase, with the sheet name's spaces as `_`
pub struct RunFiles {
    /// The CSV file uploaded to DKAN
    pub csv: String,
    /// The run log; see [`crate::run_log`]
    pub log: String,
}

impl RunFiles {
    /// The files of a run starting now
    pub fn now(dataset_id: &str, excel_sheet_name: &str) -> Self {
        let timestamp = get_local_datetime_with_format("%Y-%m-%d_%H-%M-%S");
        Self::at(dataset_id, excel_sheet_name, &timestamp)
    }

    /// The files of a run starting at `timestamp`
    pub fn at(dataset_id: &str, excel_sheet_name: &str, timestamp: &str) -> Self {
        let excel_sheet_name = normalize_string(excel_sheet_name).replace(' ', "_");
        let stem = format!("{excel_sheet_name}_{timestamp}_{dataset_id}").to_lowercase();
        RunFiles {
            csv: format!("{stem}.csv"),
            log: format!("{stem}.run.log"),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_run_log_is_named_like_the_csv_file_of_the_same_run() {
        let files = super::RunFiles::at(
            "1234567890",
            "Test Sheet with Spaces",
            "2026-11-05_10-00-00",
        );

        let expected_csv = "test_sheet_with_spaces_2026-11-05_10-00-00_1234567890.csv";
        let expected_log = "test_sheet_with_spaces_2026-11-05_10-00-00_1234567890.run.log";
        assert_eq!(files.csv, expected_csv);
        assert_eq!(files.log, expected_log);
    }
}
