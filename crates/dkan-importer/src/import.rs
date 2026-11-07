//! The import itself: read the Excel file, validate it against DKAN's data dictionary, and upload
//! it as a distribution of a dataset. Everything outside the importer, DKAN included, is reached
//! through [`Environment`], so tests run whole imports against fakes.

use crate::dkan::{Dkan, HttpsUrl, NewDistribution};
use crate::model::DataDictionary;
use crate::run_log::{RunLog, changes_lines, ng_per_ml_rows_lines, questions_lines};
use excel_core::anyhow::Result;
use excel_core::{ExcelValidator, ExcelValidatorBuilder, RowsRefused, Unpivot};
use std::collections::HashSet;
use std::io::{self, Write};

/// The heading of the validation report in the error log
pub const VALIDATION_REPORT_TITLE: &str = "Excel Validation Error Report";

/// The sheets an import reads, in the order given: at least one, and none twice. Several are
/// merged into one CSV file, each read and validated on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetNames {
    first: String,
    rest: Vec<String>,
}

impl SheetNames {
    /// Refused when `names` is empty or names a sheet more than once
    pub fn new(names: Vec<String>) -> std::result::Result<Self, String> {
        let mut seen = HashSet::new();
        if let Some(twice) = names.iter().find(|name| !seen.insert(name.as_str())) {
            return Err(format!("The sheet \"{twice}\" is given more than once."));
        }
        let mut names = names.into_iter();
        let Some(first) = names.next() else {
            return Err("No sheet given.".to_string());
        };
        Ok(SheetNames {
            first,
            rest: names.collect(),
        })
    }

    pub fn first(&self) -> &str {
        &self.first
    }

    /// Every sheet, in the order given
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.first.as_str()).chain(self.rest.iter().map(String::as_str))
    }

    /// How many sheets are merged, when more than one is given
    pub fn merged(&self) -> Option<usize> {
        (!self.rest.is_empty()).then(|| self.rest.len().saturating_add(1))
    }
}

/// What an import is given. Never a credential: those stay with the [`Environment`].
#[derive(Debug, Clone)]
pub struct Import {
    /// The path of the Excel file, as given
    pub excel_file: String,
    pub sheet_names: SheetNames,
    pub data_dictionary_id: String,
    pub dataset_id: String,
    /// The DKAN base URL, as given: refused unless https, before anything else is done
    pub base_url: String,
    pub username: String,
    /// The name the CSV is uploaded as; see [`crate::utils::RunFiles`]
    pub csv_file_name: String,
    pub unpivot: Option<Unpivot>,
    /// The field the sheet's extra columns move into; see
    /// [`ExcelValidatorBuilder::extra_columns_to`]
    pub extra_columns_to: Option<String>,
}

/// Everything an import reads or writes outside the importer, apart from the run log
pub trait Environment {
    type Dkan: Dkan;

    /// The bytes of the Excel file at `path`
    fn read_excel(&mut self, path: &str) -> io::Result<Vec<u8>>;

    /// DKAN at `base_url`, logged in as `username`: asking for the password, unless it was given
    fn connect(&mut self, base_url: HttpsUrl, username: &str) -> Result<Self::Dkan>;

    /// Save `csv`, the CSV file uploaded, as `file_name`, beside the run log
    fn save_csv(&mut self, file_name: &str, csv: &[u8]) -> io::Result<()>;

    /// Append `details` to the error log under `title`
    fn append_error_log(&mut self, title: &str, details: &str) -> io::Result<()>;
}

/// In place of a name the run stopped before reaching DKAN for
const DKAN_NOT_REACHED: &str = "the run stopped before reaching DKAN";

/// A name read from DKAN for the top of the run log
enum Name {
    Read(String),
    /// Why not
    NotRead(String),
}

impl std::fmt::Display for Name {
    /// One line, whatever the reason the name was not read
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Name::Read(name) => f.write_str(name),
            Name::NotRead(why) => write!(
                f,
                "(not read: {})",
                why.split_whitespace().collect::<Vec<_>>().join(" ")
            ),
        }
    }
}

/// The name of the file at `path`, without its directories, whether separated by `/` or `\\`
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

/// The top of the run log: the file and each sheet, then the dataset and the data dictionary,
/// each with its name, then two blank lines
fn header(import: &Import, dataset_name: &Name, dictionary_name: &Name) -> Vec<String> {
    let mut lines = vec![format!("Excel file: {}", file_name(&import.excel_file))];
    lines.extend(
        import
            .sheet_names
            .iter()
            .map(|sheet| format!("Excel sheet: {sheet}")),
    );
    lines.extend([
        format!("Dataset Name: {dataset_name}"),
        format!("Dataset ID: {}", import.dataset_id),
        format!("Dictionary Name: {dictionary_name}"),
        format!("Dictionary ID: {}", import.data_dictionary_id),
        String::new(),
        String::new(),
    ]);
    lines
}

/// The steps before the top of the run log can name the dataset and the data dictionary; they
/// log nothing
struct Reached<D> {
    dkan: D,
    workbook: Vec<u8>,
    dataset_name: Name,
    /// Needed to validate the rows, so stopping the run once the top of the log is written
    data_dictionary: Result<DataDictionary>,
}

fn reach_dkan<E: Environment>(
    import: &Import,
    environment: &mut E,
) -> std::result::Result<Reached<E::Dkan>, Box<dyn std::error::Error>> {
    // Before asking for anything: the login is sent with basic auth
    let base_url: HttpsUrl = import.base_url.parse()?;
    // Before asking for the password
    let workbook = environment
        .read_excel(&import.excel_file)
        .map_err(|e| format!("Cannot read the Excel file \"{}\": {e}", import.excel_file))?;

    let dkan = environment.connect(base_url, &import.username)?;
    // A bad login must stop the run before anything is written
    dkan.check_login()?;
    // Only for the run log: a dataset that cannot be read stops the run when it is added to
    let dataset_name = match dkan.dataset_title(&import.dataset_id) {
        Ok(title) => Name::Read(title),
        Err(e) => Name::NotRead(e.to_string()),
    };
    let data_dictionary = dkan.data_dictionary(&import.data_dictionary_id);
    Ok(Reached {
        dkan,
        workbook,
        dataset_name,
        data_dictionary,
    })
}

/// How far an import got, for the last lines of the run
enum Progress {
    NothingGenerated,
    /// The CSV file generated, but not saved, so not uploaded
    CsvNotSaved {
        rows: usize,
    },
    /// The CSV file generated and saved
    CsvGenerated {
        rows: usize,
    },
    /// The CSV file uploaded, not yet added to the dataset
    Uploaded {
        rows: usize,
        file_url: String,
    },
    Added {
        rows: usize,
        dataset_title: String,
        replaced: Replaced,
    },
}

/// The CSV distribution replaced by the one added
enum Replaced {
    Nothing,
    /// Its file, deleted
    Deleted(String),
    /// Its file, not deleted (yet)
    NotDeleted(String),
}

/// The last two lines of every run: whether the CSV file was generated, under which name, and
/// whether it was uploaded and added to the dataset
fn closing_lines(import: &Import, progress: &Progress) -> [String; 2] {
    let name = &import.csv_file_name;
    let csv = match progress {
        Progress::NothingGenerated => "❌ CSV file not generated".to_string(),
        Progress::CsvNotSaved { rows } => {
            format!("❌ CSV file generated ({rows} row(s)), but not saved as {name}")
        }
        Progress::CsvGenerated { rows }
        | Progress::Uploaded { rows, .. }
        | Progress::Added { rows, .. } => format!("✅ CSV file saved: {name} ({rows} row(s))"),
    };
    let upload = match progress {
        Progress::NothingGenerated
        | Progress::CsvNotSaved { .. }
        | Progress::CsvGenerated { .. } => "❌ Nothing uploaded".to_string(),
        Progress::Uploaded { file_url, .. } => format!(
            "❌ Uploaded {name} to {file_url}, but not added to dataset {}",
            import.dataset_id
        ),
        Progress::Added {
            dataset_title,
            replaced,
            ..
        } => {
            let added = format!(
                "Uploaded {name} and added it to dataset \"{dataset_title}\" ({})",
                import.dataset_id
            );
            match replaced {
                Replaced::Nothing => format!("✅ {added} as a new distribution"),
                Replaced::Deleted(previous) => {
                    format!("✅ {added}, replacing {previous}, whose file was deleted")
                }
                Replaced::NotDeleted(previous) => {
                    format!("⚠️  {added}, replacing {previous}, whose file could not be deleted")
                }
            }
        }
    };
    [csv, upload]
}

/// Run `import`, logging every step to `log`, then the error that stopped it, if any, and last
/// what became of the CSV file and of the upload. Stops at the first failure, before anything is
/// uploaded when the file, the login, the data dictionary or the rows are refused. Whether it
/// succeeded; an error only when the log cannot be written.
pub fn run<E: Environment, L: Write, O: Write, R: Write>(
    import: &Import,
    environment: &mut E,
    log: &mut RunLog<L, O, R>,
) -> io::Result<bool> {
    let mut progress = Progress::NothingGenerated;
    let result = steps(import, environment, log, &mut progress);
    if let Err(e) = &result {
        log.error(&format!("❌ {e}"))?;
    }
    log.info("")?;
    for line in closing_lines(import, &progress) {
        log.info(&line)?;
    }
    Ok(result.is_ok())
}

/// The steps of `import`, recording in `progress` how far it got
fn steps<E: Environment, L: Write, O: Write, R: Write>(
    import: &Import,
    environment: &mut E,
    log: &mut RunLog<L, O, R>,
    progress: &mut Progress,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let reached = reach_dkan(import, environment);
    let not_reached = || Name::NotRead(DKAN_NOT_REACHED.to_string());
    let (dataset_name, dictionary_name) = match &reached {
        Ok(reached) => (
            &reached.dataset_name,
            match &reached.data_dictionary {
                Ok(dictionary) => Name::Read(dictionary.name.clone()),
                Err(e) => Name::NotRead(e.to_string()),
            },
        ),
        Err(_) => (&not_reached(), not_reached()),
    };
    log.notes(&header(import, dataset_name, &dictionary_name))?;

    let Reached {
        dkan,
        workbook,
        data_dictionary,
        ..
    } = reached?;
    let data_dictionary = data_dictionary?;
    let json_schema = data_dictionary.to_json_schema()?;
    let title_to_name_mapping =
        DataDictionary::create_title_to_name_mapping(&data_dictionary.fields)?;

    let merged = import.sheet_names.merged();
    let mut sheets = Vec::new();
    for sheet_name in import.sheet_names.iter() {
        if merged.is_some() {
            log.info(&format!("Sheet \"{sheet_name}\":"))?;
        }
        let mut builder =
            ExcelValidatorBuilder::new(workbook.clone(), sheet_name, json_schema.clone());
        if let Some(unpivot) = &import.unpivot {
            builder = builder.unpivot(unpivot.clone());
        }
        if let Some(field) = &import.extra_columns_to {
            builder = builder.extra_columns_to(field);
        }
        let validator = builder.build()?;
        log_sheet(&validator, log)?;
        sheets.push(validator);
    }
    let rows = sheets
        .iter()
        .map(|sheet| sheet.rows().len())
        .fold(0_usize, usize::saturating_add);
    if let Some(count) = merged {
        // Before validating: rows of sheets that cannot be merged are not worth correcting yet
        ExcelValidator::check_same_columns(&sheets)?;
        log.info(&format!(
            "✅ Merging {count} sheets into one CSV file: {rows} row(s)"
        ))?;
    }

    // Every sheet validated before stopping, so one run lists all there is to correct
    let mut refusals = Vec::new();
    for sheet in &mut sheets {
        if let Err(refused) = sheet.validate_excel() {
            let title = match merged {
                None => VALIDATION_REPORT_TITLE.to_string(),
                Some(_) => format!(
                    "{VALIDATION_REPORT_TITLE}: sheet \"{}\"",
                    sheet.sheet_name()
                ),
            };
            let message = refusal_message(environment, &title, &refused);
            refusals.push(match merged {
                None => message,
                Some(_) => format!("Sheet \"{}\": {message}", sheet.sheet_name()),
            });
        }
    }
    if !refusals.is_empty() {
        return Err(refusals.join("\n").into());
    }
    log.info("✅ Validation completed!")?;

    let csv = ExcelValidator::merged_csv(&sheets, &title_to_name_mapping)
        .map_err(|e| format!("Failed to create CSV with error: {e}"))?;
    *progress = Progress::CsvNotSaved { rows };
    // Before the upload, so that the file sent is kept even if the upload fails
    environment
        .save_csv(&import.csv_file_name, &csv)
        .map_err(|e| format!("Cannot save the CSV file \"{}\": {e}", import.csv_file_name))?;
    *progress = Progress::CsvGenerated { rows };

    let file_url = dkan.upload_csv(&import.csv_file_name, csv)?;
    *progress = Progress::Uploaded {
        rows,
        file_url: file_url.clone(),
    };
    let added = dkan.add_distribution(
        &import.dataset_id,
        NewDistribution {
            file_name: &import.csv_file_name,
            file_url: &file_url,
            data_dictionary_url: &data_dictionary.url,
        },
    )?;
    *progress = Progress::Added {
        rows,
        dataset_title: added.dataset_title,
        replaced: added
            .previous_csv_filename
            .clone()
            .map_or(Replaced::Nothing, Replaced::NotDeleted),
    };

    // The CSV it replaced is no longer used
    if let Some(previous) = added.previous_csv_filename
        && let Progress::Added { replaced, .. } = progress
    {
        dkan.delete_file(&previous)?;
        *replaced = Replaced::Deleted(previous);
    }
    Ok(())
}

/// Log what reading the sheet of `validator` did: its summaries, the ng/mL rows skipped, every
/// change made to a cell, and the questions to the data owners
fn log_sheet<L: Write, O: Write, R: Write>(
    validator: &ExcelValidator,
    log: &mut RunLog<L, O, R>,
) -> io::Result<()> {
    if let Some(summary) = validator.error_cells_summary() {
        log.info(&format!("⚠️  {summary}"))?;
    }
    if let Some(summary) = validator.unpivot_summary() {
        log.info(&format!("✅ {summary}"))?;
    }
    if let Some(summary) = validator.extra_columns_summary() {
        log.info(&format!("✅ {summary}"))?;
    }
    log.notes(&ng_per_ml_rows_lines(validator.ng_per_ml_rows()))?;
    log.notes(&changes_lines(&validator.change_lines()))?;
    // Printed as well as logged: the data owners have to answer them
    for line in questions_lines(validator.questions()) {
        log.info(&line)?;
    }
    Ok(())
}

/// The message of `refused`, its report of the rows failing validation, if any, appended to the
/// error log under `title`. If the log cannot be written, the message carries the report rather
/// than pointing the user at a log that lacks it.
fn refusal_message<E: Environment>(
    environment: &mut E,
    title: &str,
    refused: &RowsRefused,
) -> String {
    let mut message = refused.to_string();
    if let Some(invalid) = &refused.invalid_rows {
        match environment.append_error_log(title, &invalid.report) {
            Ok(()) => message.push_str(" Check the error log for details."),
            Err(e) => message.push_str(&format!(
                " The error log could not be written ({e}), so the details follow:\n{}",
                invalid.report
            )),
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn sheet_names_keep_the_order_given() {
        let sheets = SheetNames::new(names(&["Water", "Air", "Sediment"])).unwrap();

        assert_eq!(sheets.first(), "Water");
        assert_eq!(
            sheets.iter().collect::<Vec<_>>(),
            vec!["Water", "Air", "Sediment"]
        );
        assert_eq!(sheets.merged(), Some(3));
    }

    #[test]
    fn one_sheet_is_not_a_merge() {
        let sheets = SheetNames::new(names(&["Water"])).unwrap();

        assert_eq!(sheets.iter().collect::<Vec<_>>(), vec!["Water"]);
        assert_eq!(sheets.merged(), None);
    }

    #[test]
    fn no_sheet_is_refused() {
        assert_eq!(
            SheetNames::new(Vec::new()),
            Err("No sheet given.".to_string())
        );
    }

    /// Its rows would be uploaded twice
    #[test]
    fn a_sheet_given_twice_is_refused() {
        let refused = SheetNames::new(names(&["Water", "Air", "Water"]));

        let expected = "The sheet \"Water\" is given more than once.";
        assert_eq!(refused, Err(expected.to_string()));
    }

    /// Whatever the path, relative or absolute, with `/` or `\\`
    #[test]
    fn the_file_name_is_the_path_without_its_directories() {
        let cases = [
            (
                "Sample_Collection&Results_Arctic 1.xlsx",
                "Sample_Collection&Results_Arctic 1.xlsx",
            ),
            (
                "data/Sample_Collection&Results_Arctic 1.xlsx",
                "Sample_Collection&Results_Arctic 1.xlsx",
            ),
            ("./data/samples.xlsx", "samples.xlsx"),
            ("../../data/2026/samples.xlsx", "samples.xlsx"),
            ("/home/user/data/samples.xlsx", "samples.xlsx"),
            ("~/data/samples.xlsx", "samples.xlsx"),
            ("data\\samples.xlsx", "samples.xlsx"),
            ("C:\\Users\\user\\Desktop\\samples.xlsx", "samples.xlsx"),
            ("data/2026\\samples.xlsx", "samples.xlsx"),
            ("data/", "data"),
        ];
        for (path, expected) in cases {
            assert_eq!(file_name(path), expected, "{path}");
        }
    }

    /// A reason such as an HTML error page keeps the name one line
    #[test]
    fn a_name_not_read_stays_one_line_whatever_the_reason() {
        let name = Name::NotRead(
            "Failed to get dataset ds-1: <html>\n  <body>Not found</body>\n</html>".to_string(),
        );

        let expected =
            "(not read: Failed to get dataset ds-1: <html> <body>Not found</body> </html>)";
        assert_eq!(name.to_string(), expected);
    }
}
