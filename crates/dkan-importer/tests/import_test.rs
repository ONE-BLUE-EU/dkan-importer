//! Whole imports against a fake DKAN and a fake environment: no network, no files, no terminal.

use dkan_importer::dkan::{DistributionAdded, Dkan, HttpsUrl, NewDistribution};
use dkan_importer::import::{Environment, Import, VALIDATION_REPORT_TITLE, run};
use dkan_importer::model::DataDictionary;
use dkan_importer::run_log::RunLog;
use excel_core::anyhow::{self, Result};
use excel_core::serde_json::{Value, json};
use excel_core::test_utils::Cell::{Empty, Number, Text};
use excel_core::test_utils::{Cell, test_workbook};
use excel_core::{DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, Unpivot};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;

const BASE_URL: &str = "https://dkan.example";
const CSV_FILE_NAME: &str = "samples_ds-1.csv";
const FILE_URL: &str = "https://dkan.example/files/samples_ds-1.csv";
const DICTIONARY_URL: &str =
    "https://dkan.example/api/1/metastore/schemas/data-dictionary/items/dd-1";

/// A call the import made to DKAN
#[derive(Debug, Clone, PartialEq)]
enum Call {
    CheckLogin,
    DataDictionary(String),
    Upload {
        file_name: String,
        csv: String,
    },
    AddDistribution {
        dataset_id: String,
        file_name: String,
        file_url: String,
        data_dictionary_url: String,
    },
    Delete(String),
}

/// A DKAN call that can be made to fail
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Login,
    DataDictionary,
    Upload,
    AddDistribution,
    Delete,
}

/// DKAN, recording the calls it gets
struct FakeDkan {
    calls: Rc<RefCell<Vec<Call>>>,
    /// The data dictionary's `data`, as DKAN serves it
    dictionary: Value,
    /// The CSV distribution the data dictionary already describes, if any
    previous: Option<&'static str>,
    fails: Option<Step>,
}

impl FakeDkan {
    fn call(&self, call: Call, step: Step) -> Result<()> {
        self.calls.borrow_mut().push(call);
        match self.fails {
            Some(failing) if failing == step => Err(anyhow::anyhow!("DKAN refused the {step:?}")),
            _ => Ok(()),
        }
    }
}

impl Dkan for FakeDkan {
    fn check_login(&self) -> Result<()> {
        self.call(Call::CheckLogin, Step::Login)
    }

    fn data_dictionary(&self, id: &str) -> Result<DataDictionary> {
        self.call(Call::DataDictionary(id.to_string()), Step::DataDictionary)?;
        let items = json!([{"identifier": "dd-1", "data": self.dictionary}]).to_string();
        DataDictionary::from_items(&BASE_URL.parse()?, id, &items)
    }

    fn upload_csv(&self, file_name: &str, csv: Vec<u8>) -> Result<String> {
        let call = Call::Upload {
            file_name: file_name.to_string(),
            csv: String::from_utf8(csv)?,
        };
        self.call(call, Step::Upload)?;
        Ok(FILE_URL.to_string())
    }

    fn add_distribution(
        &self,
        dataset_id: &str,
        new: NewDistribution<'_>,
    ) -> Result<DistributionAdded> {
        let call = Call::AddDistribution {
            dataset_id: dataset_id.to_string(),
            file_name: new.file_name.to_string(),
            file_url: new.file_url.to_string(),
            data_dictionary_url: new.data_dictionary_url.to_string(),
        };
        self.call(call, Step::AddDistribution)?;
        Ok(DistributionAdded {
            dataset_title: "Samples dataset".to_string(),
            previous_csv_filename: self.previous.map(String::from),
        })
    }

    fn delete_file(&self, file_name: &str) -> Result<()> {
        self.call(Call::Delete(file_name.to_string()), Step::Delete)
    }
}

/// The world outside the importer, recording what the import asks of it
struct FakeEnvironment {
    /// The Excel file's bytes; `None` when it cannot be read
    excel: Option<Vec<u8>>,
    /// Handed over once, on connecting
    dkan: Option<FakeDkan>,
    /// The base URL and user of each connection
    connections: Vec<(String, String)>,
    /// Each entry appended to the error log: its title and details
    error_log: Vec<(String, String)>,
    error_log_fails: bool,
}

impl Environment for FakeEnvironment {
    type Dkan = FakeDkan;

    fn read_excel(&mut self, path: &str) -> io::Result<Vec<u8>> {
        self.excel
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no file {path}")))
    }

    fn connect(&mut self, base_url: HttpsUrl, username: &str) -> Result<FakeDkan> {
        self.connections
            .push((base_url.to_string(), username.to_string()));
        self.dkan
            .take()
            .ok_or_else(|| anyhow::anyhow!("connected twice"))
    }

    fn append_error_log(&mut self, title: &str, details: &str) -> io::Result<()> {
        if self.error_log_fails {
            return Err(io::Error::other("disk full"));
        }
        self.error_log
            .push((title.to_string(), details.to_string()));
        Ok(())
    }
}

/// What a run did
struct Outcome {
    /// The error that stopped it, if any
    error: Option<String>,
    calls: Vec<Call>,
    log: String,
    stdout: String,
    stderr: String,
    connections: Vec<(String, String)>,
    error_log: Vec<(String, String)>,
}

/// A sample sheet's data dictionary: Sample ID (required) and Depth
fn samples_dictionary() -> Value {
    json!({
        "title": "Samples",
        "fields": [
            {"name": "sample_id", "title": "Sample ID", "type": "string",
             "constraints": {"required": true}},
            {"name": "depth", "title": "Depth", "type": "number"},
        ],
    })
}

fn samples_sheet(rows: Vec<Vec<Cell>>) -> Vec<u8> {
    let mut sheet = vec![vec![Text("Sample ID"), Text("Depth")]];
    sheet.extend(rows);
    test_workbook(&sheet)
}

fn valid_sheet() -> Vec<u8> {
    samples_sheet(vec![vec![Text("S1"), Number(1.5)], vec![Text("S2"), Empty]])
}

/// The CSV uploaded for [`valid_sheet`], headed by the dictionary names
const VALID_CSV: &str = "sample_id,depth\nS1,1.5\nS2,000000000000.000000\n";

fn import() -> Import {
    Import {
        excel_file: "samples.xlsx".to_string(),
        sheet_name: "Sheet1".to_string(),
        data_dictionary_id: "dd-1".to_string(),
        dataset_id: "ds-1".to_string(),
        base_url: BASE_URL.to_string(),
        username: "admin".to_string(),
        csv_file_name: CSV_FILE_NAME.to_string(),
        unpivot: None,
        extra_columns_to: None,
    }
}

fn environment(excel: Vec<u8>, dkan: FakeDkan) -> FakeEnvironment {
    FakeEnvironment {
        excel: Some(excel),
        dkan: Some(dkan),
        connections: Vec::new(),
        error_log: Vec::new(),
        error_log_fails: false,
    }
}

fn dkan(previous: Option<&'static str>, fails: Option<Step>) -> FakeDkan {
    FakeDkan {
        calls: Rc::new(RefCell::new(Vec::new())),
        dictionary: samples_dictionary(),
        previous,
        fails,
    }
}

fn run_import(import: &Import, mut environment: FakeEnvironment) -> Outcome {
    let calls = environment
        .dkan
        .as_ref()
        .map(|dkan| Rc::clone(&dkan.calls))
        .unwrap_or_default();
    let mut log = RunLog::new(Vec::new(), Vec::new(), Vec::new());
    let error = run(import, &mut environment, &mut log)
        .err()
        .map(|e| e.to_string());
    let (log, stdout, stderr) = log.into_parts();
    let calls = calls.borrow().clone();
    Outcome {
        error,
        calls,
        log: String::from_utf8(log).unwrap(),
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
        connections: environment.connections,
        error_log: environment.error_log,
    }
}

/// What the run is given, at the top of every run log
const NOTES: &str = "Excel file: samples.xlsx
Sheet: Sheet1
Dataset: ds-1
Data dictionary: dd-1
DKAN: https://dkan.example
";

fn upload() -> Call {
    Call::Upload {
        file_name: CSV_FILE_NAME.to_string(),
        csv: VALID_CSV.to_string(),
    }
}

fn add_distribution() -> Call {
    Call::AddDistribution {
        dataset_id: "ds-1".to_string(),
        file_name: CSV_FILE_NAME.to_string(),
        file_url: FILE_URL.to_string(),
        data_dictionary_url: DICTIONARY_URL.to_string(),
    }
}

fn data_dictionary() -> Call {
    Call::DataDictionary("dd-1".to_string())
}

#[test]
fn a_valid_sheet_is_uploaded_and_added_as_a_new_distribution() {
    let outcome = run_import(&import(), environment(valid_sheet(), dkan(None, None)));

    assert_eq!(outcome.error, None);
    let expected = vec![
        Call::CheckLogin,
        data_dictionary(),
        upload(),
        add_distribution(),
    ];
    assert_eq!(outcome.calls, expected);
    let expected_connections = vec![(BASE_URL.to_string(), "admin".to_string())];
    assert_eq!(outcome.connections, expected_connections);
    let printed = "✅ Login accepted for user \"admin\"
✅ Validation completed!
✅ CSV file created: samples_ds-1.csv
✅ Successfully added CSV distribution 'samples_ds-1.csv' to dataset \"Samples dataset\" with id \"ds-1\"
";
    assert_eq!(outcome.stdout, printed);
    let logged = format!(
        "{NOTES}✅ Login accepted for user \"admin\"
No cell changed: every cell was taken as written.
✅ Validation completed!
✅ CSV file created: samples_ds-1.csv
✅ Successfully added CSV distribution 'samples_ds-1.csv' to dataset \"Samples dataset\" with id \"ds-1\"
"
    );
    assert_eq!(outcome.log, logged);
    assert_eq!(outcome.stderr, "");
    assert!(outcome.error_log.is_empty());
}

#[test]
fn the_distribution_replaced_has_its_file_deleted_once_the_new_one_is_added() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(Some("old.csv"), None)),
    );

    assert_eq!(outcome.error, None);
    let expected = vec![
        Call::CheckLogin,
        data_dictionary(),
        upload(),
        add_distribution(),
        Call::Delete("old.csv".to_string()),
    ];
    assert_eq!(outcome.calls, expected);
    let printed = "✅ Successfully replaced CSV distribution 'old.csv' with 'samples_ds-1.csv' in dataset \"Samples dataset\" with id \"ds-1\"
🧹 Previous CSV file successfully deleted: old.csv
";
    assert!(outcome.stdout.ends_with(printed), "{}", outcome.stdout);
    assert!(outcome.log.ends_with(printed), "{}", outcome.log);
}

#[test]
fn a_url_that_is_not_https_stops_the_run_before_reading_the_file_or_connecting() {
    let mut import = import();
    import.base_url = "http://dkan.example".to_string();
    let mut environment = environment(valid_sheet(), dkan(None, None));
    // Reading the file would fail, so the URL must be refused first
    environment.excel = None;

    let outcome = run_import(&import, environment);

    let expected = "The URL must be https. The provided URL is: http://dkan.example";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    assert!(outcome.connections.is_empty());
    assert!(outcome.calls.is_empty());
    let expected_log = NOTES.replace("https://dkan.example", "http://dkan.example");
    assert_eq!(outcome.log, expected_log);
}

#[test]
fn an_unreadable_excel_file_stops_the_run_before_asking_for_the_password() {
    let mut environment = environment(valid_sheet(), dkan(None, None));
    environment.excel = None;

    let outcome = run_import(&import(), environment);

    let expected = "Cannot read the Excel file \"samples.xlsx\": no file samples.xlsx";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    assert!(outcome.connections.is_empty());
    assert!(outcome.calls.is_empty());
}

#[test]
fn a_rejected_login_stops_the_run_before_the_data_dictionary() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(None, Some(Step::Login))),
    );

    assert_eq!(outcome.error.as_deref(), Some("DKAN refused the Login"));
    assert_eq!(outcome.calls, vec![Call::CheckLogin]);
    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.log, NOTES);
}

#[test]
fn a_data_dictionary_that_cannot_be_read_stops_the_run_before_the_upload() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(None, Some(Step::DataDictionary))),
    );

    assert_eq!(
        outcome.error.as_deref(),
        Some("DKAN refused the DataDictionary")
    );
    assert_eq!(outcome.calls, vec![Call::CheckLogin, data_dictionary()]);
}

fn invalid_sheet() -> Vec<u8> {
    samples_sheet(vec![
        vec![Text("S1"), Number(1.5)],
        vec![Text("S2"), Text("deep")],
    ])
}

#[test]
fn rows_failing_validation_go_to_the_error_log_and_nothing_is_uploaded() {
    let outcome = run_import(&import(), environment(invalid_sheet(), dkan(None, None)));

    let expected =
        "Excel validation failed with 1 error(s) across 1 row(s). Check the error log for details.";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    assert_eq!(outcome.calls, vec![Call::CheckLogin, data_dictionary()]);
    let [(title, report)] = outcome.error_log.as_slice() else {
        panic!("one entry expected: {:?}", outcome.error_log);
    };
    assert_eq!(title, VALIDATION_REPORT_TITLE);
    assert!(report.contains("Row 3: 1 error(s)\n"), "{report}");
    assert!(report.contains("\"deep\""), "{report}");
}

#[test]
fn when_the_error_log_cannot_be_written_the_details_are_in_the_error() {
    let mut environment = environment(invalid_sheet(), dkan(None, None));
    environment.error_log_fails = true;

    let outcome = run_import(&import(), environment);

    let error = outcome.error.expect("the invalid row must stop the run");
    let expected = "Excel validation failed with 1 error(s) across 1 row(s). The error log could not be written (disk full), so the details follow:\n";
    assert!(error.starts_with(expected), "{error}");
    assert!(error.contains("Row 3: 1 error(s)\n"), "{error}");
    assert!(!error.contains("Check the error log"), "{error}");
    assert_eq!(outcome.calls, vec![Call::CheckLogin, data_dictionary()]);
}

/// An unpivoted analytical sheet's data dictionary
fn analytical_dictionary() -> Value {
    let field = |title: &str, kind: &str| json!({"name": title, "title": title, "type": kind});
    json!({
        "title": "Analytical results",
        "fields": [
            field("Substance", "string"),
            field("Identification Level", "string"),
            field("Detection characterisation", "string"),
            field("Concentration units", "string"),
            field("Limit of detection", "number"),
            field("Limit of quantification", "number"),
            field("Sample Tag", "string"),
            field("Concentration", "string"),
        ],
    })
}

fn unpivot() -> Unpivot {
    Unpivot {
        headers_column: "Sample Tag".to_string(),
        values_column: "Concentration".to_string(),
        values_markers: DEFAULT_VALUES_MARKERS.map(String::from).to_vec(),
        marker_limits: DEFAULT_MARKER_LIMITS
            .iter()
            .map(|limit| limit.parse().unwrap())
            .collect(),
    }
}

#[test]
fn an_ng_ml_row_without_its_ng_l_row_is_in_the_run_log_and_nothing_is_uploaded() {
    let sheet = test_workbook(&[
        vec![
            Text("Substance"),
            Text("Identification Level"),
            Text("Detection characterisation"),
            Text("Concentration units"),
            Text("Limit of detection"),
            Text("Limit of quantification"),
            Text("ST01"),
        ],
        vec![
            Text("Okadaic acid"),
            Text("2A"),
            Empty,
            Text("ng/mL"),
            Number(0.0001),
            Number(0.00025),
            Number(0.0003),
        ],
    ]);
    let mut dkan = dkan(None, None);
    dkan.dictionary = analytical_dictionary();
    let mut import = import();
    import.unpivot = Some(unpivot());

    let outcome = run_import(&import, environment(sheet, dkan));

    let error = outcome.error.expect("the ng/mL row must stop the run");
    assert!(
        error
            .starts_with("The sheet \"Sheet1\" has 1 ng/mL row(s) whose values no ng/L row holds."),
        "{error}"
    );
    let logged = "1 ng/mL row(s) skipped, as only ng/L is imported, each with the ng/L row holding its values:
  row 2: skipped: in ng/mL, but the sheet has no ng/L row to hold its values
";
    assert!(outcome.log.contains(logged), "{}", outcome.log);
    assert!(
        outcome.stdout.contains("0 row(s); 1 ng/mL row(s) skipped"),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.calls, vec![Call::CheckLogin, data_dictionary()]);
    assert!(outcome.error_log.is_empty());
}

#[test]
fn a_failed_upload_stops_the_run_before_the_distribution_is_added() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(Some("old.csv"), Some(Step::Upload))),
    );

    assert_eq!(outcome.error.as_deref(), Some("DKAN refused the Upload"));
    let expected = vec![Call::CheckLogin, data_dictionary(), upload()];
    assert_eq!(outcome.calls, expected);
}

#[test]
fn a_distribution_not_added_stops_the_run_before_the_previous_file_is_deleted() {
    let outcome = run_import(
        &import(),
        environment(
            valid_sheet(),
            dkan(Some("old.csv"), Some(Step::AddDistribution)),
        ),
    );

    assert_eq!(
        outcome.error.as_deref(),
        Some("DKAN refused the AddDistribution")
    );
    let expected = vec![
        Call::CheckLogin,
        data_dictionary(),
        upload(),
        add_distribution(),
    ];
    assert_eq!(outcome.calls, expected);
}

#[test]
fn a_previous_file_not_deleted_stops_the_run_after_the_distribution_is_added() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(Some("old.csv"), Some(Step::Delete))),
    );

    assert_eq!(outcome.error.as_deref(), Some("DKAN refused the Delete"));
    assert_eq!(
        outcome.calls.last(),
        Some(&Call::Delete("old.csv".to_string()))
    );
    assert!(
        outcome
            .log
            .contains("✅ Successfully replaced CSV distribution 'old.csv'"),
        "{}",
        outcome.log
    );
    assert!(!outcome.log.contains("🧹"), "{}", outcome.log);
}
