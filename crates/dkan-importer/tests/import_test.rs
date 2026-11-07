//! Whole imports against a fake DKAN and a fake environment: no network, no files, no terminal.

use dkan_importer::dkan::{DistributionAdded, Dkan, HttpsUrl, NewDistribution};
use dkan_importer::import::{Environment, Import, SheetNames, VALIDATION_REPORT_TITLE, run};
use dkan_importer::model::DataDictionary;
use dkan_importer::run_log::RunLog;
use excel_core::anyhow::{self, Result};
use excel_core::serde_json::{Value, json};
use excel_core::test_utils::Cell::{Empty, Number, Text};
use excel_core::test_utils::{Cell, test_workbook, test_workbook_with_sheets};
use excel_core::{DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, Unpivot};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;

const BASE_URL: &str = "https://dkan.example";
const DATASET_TITLE: &str = "Samples dataset";
const CSV_FILE_NAME: &str = "samples_ds-1.csv";
const FILE_URL: &str = "https://dkan.example/files/samples_ds-1.csv";
const DICTIONARY_URL: &str =
    "https://dkan.example/api/1/metastore/schemas/data-dictionary/items/dd-1";

/// A call the import made to DKAN
#[derive(Debug, Clone, PartialEq)]
enum Call {
    CheckLogin,
    DatasetTitle(String),
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
    DatasetTitle,
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

    fn dataset_title(&self, dataset_id: &str) -> Result<String> {
        self.call(
            Call::DatasetTitle(dataset_id.to_string()),
            Step::DatasetTitle,
        )?;
        Ok(DATASET_TITLE.to_string())
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
            dataset_title: DATASET_TITLE.to_string(),
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
    /// Each CSV file saved: its name and contents
    saved_csv: Vec<(String, String)>,
    saving_csv_fails: bool,
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

    fn save_csv(&mut self, file_name: &str, csv: &[u8]) -> io::Result<()> {
        if self.saving_csv_fails {
            return Err(io::Error::other("disk full"));
        }
        self.saved_csv.push((
            file_name.to_string(),
            String::from_utf8_lossy(csv).to_string(),
        ));
        Ok(())
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
    /// The error that stopped it, if any, as printed
    error: Option<String>,
    calls: Vec<Call>,
    log: String,
    stdout: String,
    stderr: String,
    connections: Vec<(String, String)>,
    error_log: Vec<(String, String)>,
    saved_csv: Vec<(String, String)>,
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
        sheet_names: sheet_names(&["Sheet1"]),
        data_dictionary_id: "dd-1".to_string(),
        dataset_id: "ds-1".to_string(),
        base_url: BASE_URL.to_string(),
        username: "admin".to_string(),
        csv_file_name: CSV_FILE_NAME.to_string(),
        unpivot: None,
        extra_columns_to: None,
    }
}

fn sheet_names(names: &[&str]) -> SheetNames {
    SheetNames::new(names.iter().map(|name| name.to_string()).collect()).unwrap()
}

fn environment(excel: Vec<u8>, dkan: FakeDkan) -> FakeEnvironment {
    FakeEnvironment {
        excel: Some(excel),
        dkan: Some(dkan),
        connections: Vec::new(),
        error_log: Vec::new(),
        error_log_fails: false,
        saved_csv: Vec::new(),
        saving_csv_fails: false,
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
    let succeeded = run(import, &mut environment, &mut log).expect("the run log must be written");
    let (log, stdout, stderr) = log.into_parts();
    let stderr = String::from_utf8(stderr).unwrap();
    // The run prints the error that stopped it, and only that, to the standard error
    let error = stderr
        .strip_prefix("❌ ")
        .map(|error| error.strip_suffix('\n').unwrap_or(error).to_string());
    assert_eq!(error.is_none(), stderr.is_empty(), "{stderr}");
    assert_eq!(succeeded, error.is_none(), "{stderr}");
    let calls = calls.borrow().clone();
    Outcome {
        error,
        calls,
        log: String::from_utf8(log).unwrap(),
        stdout: String::from_utf8(stdout).unwrap(),
        stderr,
        connections: environment.connections,
        error_log: environment.error_log,
        saved_csv: environment.saved_csv,
    }
}

/// The top of the run log, with the dataset's and the data dictionary's names as shown, followed
/// by two blank lines
fn header(dataset_name: &str, dictionary_name: &str) -> String {
    format!(
        "Excel file: samples.xlsx
Excel sheet: Sheet1
Dataset Name: {dataset_name}
Dataset ID: ds-1
Dictionary Name: {dictionary_name}
Dictionary ID: dd-1


"
    )
}

/// The dataset's and the data dictionary's names, as DKAN gives them
fn header_with_names() -> String {
    header(DATASET_TITLE, "Samples")
}

/// In place of a name, when the run stopped before DKAN could give it
const NOT_REACHED: &str = "(not read: the run stopped before reaching DKAN)";

/// The last lines of every run: what became of the CSV file and of the upload
fn closing(csv: &str, upload: &str) -> String {
    format!("\n{csv}\n{upload}\n")
}

const CSV_GENERATED: &str = "✅ CSV file saved: samples_ds-1.csv (2 row(s))";
const CSV_NOT_GENERATED: &str = "❌ CSV file not generated";
const NOTHING_UPLOADED: &str = "❌ Nothing uploaded";

/// The last lines of a run stopped before the CSV file was generated
fn nothing_done() -> String {
    closing(CSV_NOT_GENERATED, NOTHING_UPLOADED)
}

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

fn dataset_title() -> Call {
    Call::DatasetTitle("ds-1".to_string())
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
        dataset_title(),
        data_dictionary(),
        upload(),
        add_distribution(),
    ];
    assert_eq!(outcome.calls, expected);
    let expected_connections = vec![(BASE_URL.to_string(), "admin".to_string())];
    assert_eq!(outcome.connections, expected_connections);
    let closing = closing(
        CSV_GENERATED,
        "✅ Uploaded samples_ds-1.csv and added it to dataset \"Samples dataset\" (ds-1) as a new distribution",
    );
    let printed = format!("✅ Validation completed!\n{closing}");
    assert_eq!(outcome.stdout, printed);
    let logged = format!(
        "{}Every cell was imported as written.\n{printed}",
        header_with_names()
    );
    assert_eq!(outcome.log, logged);
    assert_eq!(outcome.stderr, "");
    assert!(outcome.error_log.is_empty());
    let expected = vec![(CSV_FILE_NAME.to_string(), VALID_CSV.to_string())];
    assert_eq!(outcome.saved_csv, expected);
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
        dataset_title(),
        data_dictionary(),
        upload(),
        add_distribution(),
        Call::Delete("old.csv".to_string()),
    ];
    assert_eq!(outcome.calls, expected);
    let printed = closing(
        CSV_GENERATED,
        "✅ Uploaded samples_ds-1.csv and added it to dataset \"Samples dataset\" (ds-1), replacing old.csv, whose file was deleted",
    );
    assert!(outcome.stdout.ends_with(&printed), "{}", outcome.stdout);
    assert!(outcome.log.ends_with(&printed), "{}", outcome.log);
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
    let logged = format!(
        "{}❌ {expected}\n{}",
        header(NOT_REACHED, NOT_REACHED),
        nothing_done()
    );
    assert_eq!(outcome.log, logged);
    assert_eq!(outcome.stdout, nothing_done());
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
    let logged = format!(
        "{}❌ {expected}\n{}",
        header(NOT_REACHED, NOT_REACHED),
        nothing_done()
    );
    assert_eq!(outcome.log, logged);
}

#[test]
fn a_rejected_login_stops_the_run_before_the_data_dictionary() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(None, Some(Step::Login))),
    );

    assert_eq!(outcome.error.as_deref(), Some("DKAN refused the Login"));
    assert_eq!(outcome.calls, vec![Call::CheckLogin]);
    assert_eq!(outcome.stdout, nothing_done());
    let logged = format!(
        "{}❌ DKAN refused the Login\n{}",
        header(NOT_REACHED, NOT_REACHED),
        nothing_done()
    );
    assert_eq!(outcome.log, logged);
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
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
    let expected = format!(
        "{}❌ DKAN refused the DataDictionary\n{}",
        header(DATASET_TITLE, "(not read: DKAN refused the DataDictionary)"),
        nothing_done()
    );
    assert_eq!(outcome.log, expected);
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
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
    let [(title, report)] = outcome.error_log.as_slice() else {
        panic!("one entry expected: {:?}", outcome.error_log);
    };
    assert_eq!(title, VALIDATION_REPORT_TITLE);
    assert!(report.contains("Row 3: 1 error(s)\n"), "{report}");
    assert!(report.contains("\"deep\""), "{report}");
    assert!(
        outcome.stdout.ends_with(&nothing_done()),
        "{}",
        outcome.stdout
    );
    assert!(outcome.log.ends_with(&nothing_done()), "{}", outcome.log);
    assert!(outcome.saved_csv.is_empty());
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
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
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

/// Only an ng/mL row whose values an ng/L row holds is skipped
#[test]
fn an_ng_ml_row_without_its_ng_l_row_is_logged_and_uploaded_in_ng_ml() {
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
            Number(2.0),
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

    assert_eq!(outcome.error, None);
    let logged = "1 row(s) in units also held converted (ng/mL as ng/L, µg/m3 as ng/m3, µg/g as ng/g): 0 skipped, as another row holds their values, and 1 imported as written, as none does:
  row 2: imported in ng/mL, as the sheet has no ng/L row to hold its values
";
    assert!(outcome.log.contains(logged), "{}", outcome.log);
    assert!(outcome.stdout.contains(": 1 row(s)."), "{}", outcome.stdout);
    let csv = "Substance,Identification Level,Detection characterisation,Concentration units,Limit of detection,Limit of quantification,Sample Tag,Concentration
Okadaic acid,2,detected quantitatively,ng/mL,0.0001,0.00025,ST01,0.0003
";
    let [(_, saved)] = outcome.saved_csv.as_slice() else {
        panic!("one CSV file expected: {:?}", outcome.saved_csv);
    };
    assert_eq!(saved, csv);
    assert!(
        outcome
            .calls
            .iter()
            .any(|call| matches!(call, Call::Upload { .. })),
        "{:?}",
        outcome.calls
    );
}

#[test]
fn a_failed_upload_stops_the_run_before_the_distribution_is_added() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(Some("old.csv"), Some(Step::Upload))),
    );

    assert_eq!(outcome.error.as_deref(), Some("DKAN refused the Upload"));
    let expected = vec![
        Call::CheckLogin,
        dataset_title(),
        data_dictionary(),
        upload(),
    ];
    assert_eq!(outcome.calls, expected);
    let printed = closing(CSV_GENERATED, NOTHING_UPLOADED);
    assert!(outcome.stdout.ends_with(&printed), "{}", outcome.stdout);
    assert!(outcome.log.ends_with(&printed), "{}", outcome.log);
    // Saved before the upload, so the file sent is there to open
    let expected = vec![(CSV_FILE_NAME.to_string(), VALID_CSV.to_string())];
    assert_eq!(outcome.saved_csv, expected);
}

/// Nothing is uploaded that is not also saved
#[test]
fn a_csv_file_that_cannot_be_saved_stops_the_run_before_the_upload() {
    let mut environment = environment(valid_sheet(), dkan(None, None));
    environment.saving_csv_fails = true;

    let outcome = run_import(&import(), environment);

    let expected = "Cannot save the CSV file \"samples_ds-1.csv\": disk full";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    let expected = vec![Call::CheckLogin, dataset_title(), data_dictionary()];
    assert_eq!(outcome.calls, expected);
    let printed = closing(
        "❌ CSV file generated (2 row(s)), but not saved as samples_ds-1.csv",
        NOTHING_UPLOADED,
    );
    assert!(outcome.stdout.ends_with(&printed), "{}", outcome.stdout);
    assert!(outcome.log.ends_with(&printed), "{}", outcome.log);
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
        dataset_title(),
        data_dictionary(),
        upload(),
        add_distribution(),
    ];
    assert_eq!(outcome.calls, expected);
    let printed = closing(
        CSV_GENERATED,
        "❌ Uploaded samples_ds-1.csv to https://dkan.example/files/samples_ds-1.csv, but not added to dataset ds-1",
    );
    assert!(outcome.stdout.ends_with(&printed), "{}", outcome.stdout);
    assert!(outcome.log.ends_with(&printed), "{}", outcome.log);
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
    let printed = closing(
        CSV_GENERATED,
        "⚠️  Uploaded samples_ds-1.csv and added it to dataset \"Samples dataset\" (ds-1), replacing old.csv, whose file could not be deleted",
    );
    assert!(outcome.stdout.ends_with(&printed), "{}", outcome.stdout);
    assert!(outcome.log.ends_with(&printed), "{}", outcome.log);
}

/// The name is only for the run log: a dataset whose name cannot be read is still imported into,
/// the reason taking the name's place
#[test]
fn a_dataset_name_that_cannot_be_read_is_explained_at_the_top_and_the_run_goes_on() {
    let outcome = run_import(
        &import(),
        environment(valid_sheet(), dkan(None, Some(Step::DatasetTitle))),
    );

    assert_eq!(outcome.error, None);
    let expected = format!(
        "{}Every cell was imported as written.\n",
        header("(not read: DKAN refused the DatasetTitle)", "Samples")
    );
    assert!(outcome.log.starts_with(&expected), "{}", outcome.log);
    let expected = vec![
        Call::CheckLogin,
        dataset_title(),
        data_dictionary(),
        upload(),
        add_distribution(),
    ];
    assert_eq!(outcome.calls, expected);
}

/// The top of the log names the dataset and the data dictionary in every run that reached DKAN,
/// even one stopped later
#[test]
fn the_top_of_the_log_has_the_names_in_a_run_stopped_after_reaching_dkan() {
    let outcome = run_import(&import(), environment(invalid_sheet(), dkan(None, None)));

    assert!(outcome.error.is_some());
    assert!(
        outcome.log.starts_with(&header_with_names()),
        "{}",
        outcome.log
    );
}

/// The top of the log names the file without its directories; an error names the whole path
#[test]
fn the_top_of_the_log_names_the_file_without_its_path() {
    let mut import = import();
    import.excel_file = "data/2026/samples.xlsx".to_string();
    let mut environment = environment(valid_sheet(), dkan(None, None));
    environment.excel = None;

    let outcome = run_import(&import, environment);

    assert!(
        outcome.log.starts_with("Excel file: samples.xlsx\n"),
        "{}",
        outcome.log
    );
    let expected =
        "Cannot read the Excel file \"data/2026/samples.xlsx\": no file data/2026/samples.xlsx";
    assert_eq!(outcome.error.as_deref(), Some(expected));
}

/// The questions to the data owners are printed as well as logged, numbered
#[test]
fn the_questions_are_printed_and_logged_numbered() {
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
            Number(2.0),
            Empty,
            Text("ng/L"),
            Number(0.1),
            Number(0.25),
            Text("<LOQ"),
        ],
    ]);
    let mut dkan = dkan(None, None);
    dkan.dictionary = analytical_dictionary();
    let mut import = import();
    import.unpivot = Some(unpivot());

    let outcome = run_import(&import, environment(sheet, dkan));

    assert_eq!(outcome.error, None);
    let questions = "Questions to data owners:
Q1 (1 cell): Results between the Limit of detection and the Limit of quantification, and \"<LOQ\" results, are imported as \"detected, not quantified\". The template's dropdown list does not have this value: it has \"detected semiquantitatively\". Which should be used?
";
    assert!(outcome.stdout.contains(questions), "{}", outcome.stdout);
    assert!(outcome.log.contains(questions), "{}", outcome.log);
    assert!(!outcome.log.contains("see the run log"), "{}", outcome.log);
}

/// A workbook with sample sheets "Water", "Air" and "Reordered", whose columns are in another
/// order; with an invalid Depth in Water's row 3 and Air's row 2 when `invalid`
fn samples_workbook(invalid: bool) -> Vec<u8> {
    let headers = || vec![Text("Sample ID"), Text("Depth")];
    let depth = |value: f64| if invalid { Text("deep") } else { Number(value) };
    let water = [
        headers(),
        vec![Text("W1"), Number(1.5)],
        vec![Text("W2"), depth(2.0)],
    ];
    let air = [headers(), vec![Text("A1"), depth(0.5)]];
    let reordered = [
        vec![Text("Depth"), Text("Sample ID")],
        vec![Text("deep"), Text("R1")],
    ];
    test_workbook_with_sheets(&[("Water", &water), ("Air", &air), ("Reordered", &reordered)])
}

fn merging(names: &[&str]) -> Import {
    let mut import = import();
    import.sheet_names = sheet_names(names);
    import
}

#[test]
fn several_sheets_are_merged_into_one_csv_file_in_the_order_given() {
    let outcome = run_import(
        &merging(&["Air", "Water"]),
        environment(samples_workbook(false), dkan(None, None)),
    );

    assert_eq!(outcome.error, None);
    let csv = "sample_id,depth\nA1,0.5\nW1,1.5\nW2,2\n";
    let expected = vec![(CSV_FILE_NAME.to_string(), csv.to_string())];
    assert_eq!(outcome.saved_csv, expected);
    let expected = Call::Upload {
        file_name: CSV_FILE_NAME.to_string(),
        csv: csv.to_string(),
    };
    assert_eq!(outcome.calls.get(3), Some(&expected));
    let printed = "Sheet \"Air\":
Sheet \"Water\":
✅ Merging 2 sheets into one CSV file: 3 row(s)
✅ Validation completed!
";
    assert!(outcome.stdout.starts_with(printed), "{}", outcome.stdout);
    let logged = "Excel file: samples.xlsx
Excel sheet: Air
Excel sheet: Water
Dataset Name: Samples dataset
Dataset ID: ds-1
Dictionary Name: Samples
Dictionary ID: dd-1


Sheet \"Air\":
Every cell was imported as written.
Sheet \"Water\":
Every cell was imported as written.
✅ Merging 2 sheets into one CSV file: 3 row(s)
";
    assert!(outcome.log.starts_with(logged), "{}", outcome.log);
    assert!(
        outcome
            .log
            .contains("✅ CSV file saved: samples_ds-1.csv (3 row(s))\n"),
        "{}",
        outcome.log
    );
}

/// One run lists all there is to correct: every sheet is validated before the run stops
#[test]
fn every_sheet_refused_is_reported_and_nothing_is_uploaded() {
    let outcome = run_import(
        &merging(&["Water", "Air"]),
        environment(samples_workbook(true), dkan(None, None)),
    );

    let expected = "Sheet \"Water\": Excel validation failed with 1 error(s) across 1 row(s). Check the error log for details.
Sheet \"Air\": Excel validation failed with 1 error(s) across 1 row(s). Check the error log for details.";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    let titles: Vec<&str> = outcome
        .error_log
        .iter()
        .map(|(title, _)| title.as_str())
        .collect();
    let expected = vec![
        "Excel Validation Error Report: sheet \"Water\"",
        "Excel Validation Error Report: sheet \"Air\"",
    ];
    assert_eq!(titles, expected);
    let reports: Vec<&str> = outcome
        .error_log
        .iter()
        .map(|(_, report)| report.as_str())
        .collect();
    assert!(reports[0].contains("Row 3: 1 error(s)\n"), "{reports:?}");
    assert!(reports[1].contains("Row 2: 1 error(s)\n"), "{reports:?}");
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
    assert!(outcome.saved_csv.is_empty());
    assert!(outcome.log.ends_with(&nothing_done()), "{}", outcome.log);
}

/// One valid sheet among refused ones is not uploaded alone
#[test]
fn a_valid_sheet_is_not_uploaded_when_another_is_refused() {
    let mut workbook_import = merging(&["Water", "Air"]);
    workbook_import.sheet_names = sheet_names(&["Sheet1", "Air"]);
    let invalid = [
        vec![Text("Sample ID"), Text("Depth")],
        vec![Text("A1"), Text("deep")],
    ];
    let valid = [
        vec![Text("Sample ID"), Text("Depth")],
        vec![Text("S1"), Number(1.0)],
    ];
    let workbook = test_workbook_with_sheets(&[("Sheet1", &valid), ("Air", &invalid)]);

    let outcome = run_import(&workbook_import, environment(workbook, dkan(None, None)));

    let expected = "Sheet \"Air\": Excel validation failed with 1 error(s) across 1 row(s). Check the error log for details.";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
    assert!(outcome.saved_csv.is_empty());
}

/// Before validating: the rows of sheets that cannot be merged are not worth correcting yet
#[test]
fn sheets_whose_columns_differ_stop_the_run_before_validation() {
    let outcome = run_import(
        &merging(&["Water", "Reordered"]),
        environment(samples_workbook(false), dkan(None, None)),
    );

    let expected = "The sheet \"Reordered\" has the columns Depth, Sample ID, but the sheet \"Water\" has Sample ID, Depth: only sheets with the same columns, in the same order, can be merged.";
    assert_eq!(outcome.error.as_deref(), Some(expected));
    // Reordered's Depth "deep" would fail validation
    assert!(outcome.error_log.is_empty(), "{:?}", outcome.error_log);
    assert!(
        !outcome.log.contains("Validation completed"),
        "{}",
        outcome.log
    );
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
}

#[test]
fn a_missing_sheet_among_those_given_stops_the_run_before_the_upload() {
    let outcome = run_import(
        &merging(&["Water", "Soil"]),
        environment(samples_workbook(false), dkan(None, None)),
    );

    let error = outcome.error.expect("the missing sheet must stop the run");
    assert!(error.contains("Soil"), "{error}");
    assert_eq!(
        outcome.calls,
        vec![Call::CheckLogin, dataset_title(), data_dictionary()]
    );
    assert!(outcome.saved_csv.is_empty());
}
