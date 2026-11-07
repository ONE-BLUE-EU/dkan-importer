//! The import itself: read the Excel file, validate it against DKAN's data dictionary, and upload
//! it as a distribution of a dataset. Everything outside the importer, DKAN included, is reached
//! through [`Environment`], so tests run whole imports against fakes.

use crate::dkan::{Dkan, HttpsUrl, NewDistribution};
use crate::model::DataDictionary;
use crate::run_log::{RunLog, changes_lines, ng_per_ml_rows_lines, questions_lines};
use excel_core::anyhow::Result;
use excel_core::{ExcelValidatorBuilder, Unpivot};
use std::io::{self, Write};

/// The heading of the validation report in the error log
pub const VALIDATION_REPORT_TITLE: &str = "Excel Validation Error Report";

/// What an import is given. Never a credential: those stay with the [`Environment`].
#[derive(Debug, Clone)]
pub struct Import {
    /// The path of the Excel file, as given
    pub excel_file: String,
    pub sheet_name: String,
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

/// The top of the run log: the file and sheet, then the dataset and the data dictionary, each with
/// its name, then two blank lines
fn header(import: &Import, dataset_name: &Name, dictionary_name: &Name) -> [String; 8] {
    [
        format!("Excel file: {}", file_name(&import.excel_file)),
        format!("Excel sheet: {}", import.sheet_name),
        format!("Dataset Name: {dataset_name}"),
        format!("Dataset ID: {}", import.dataset_id),
        format!("Dictionary Name: {dictionary_name}"),
        format!("Dictionary ID: {}", import.data_dictionary_id),
        String::new(),
        String::new(),
    ]
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

/// Run `import`, logging every step to `log`. Stops at the first failure, before anything is
/// uploaded when the file, the login, the data dictionary or the rows are refused.
pub fn run<E: Environment, L: Write, O: Write, R: Write>(
    import: &Import,
    environment: &mut E,
    log: &mut RunLog<L, O, R>,
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
    log.info(&format!(
        "✅ Login accepted for user \"{}\"",
        import.username
    ))?;
    let data_dictionary = data_dictionary?;
    let json_schema = data_dictionary.to_json_schema()?;
    let title_to_name_mapping =
        DataDictionary::create_title_to_name_mapping(&data_dictionary.fields)?;

    let mut builder = ExcelValidatorBuilder::new(workbook, &import.sheet_name, json_schema);
    if let Some(unpivot) = &import.unpivot {
        builder = builder.unpivot(unpivot.clone());
    }
    if let Some(field) = &import.extra_columns_to {
        builder = builder.extra_columns_to(field);
    }
    let mut validator = builder.build()?;
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
    log.notes(&changes_lines(validator.changes()))?;
    let questions = validator.questions();
    if !questions.is_empty() {
        let cells: usize = questions
            .iter()
            .map(|(_, cells)| *cells)
            .fold(0, usize::saturating_add);
        log.info(&format!(
            "❓ {} question(s) about the data, concerning {} cell(s) not imported or imported on an assumption: see the run log",
            questions.len(),
            cells
        ))?;
        log.notes(&questions_lines(questions))?;
    }
    if let Err(refused) = validator.validate_excel() {
        let mut message = refused.to_string();
        if let Some(invalid) = &refused.invalid_rows {
            // If the log cannot be written, carry the details in the error itself rather than
            // pointing the user at a log that lacks them
            match environment.append_error_log(VALIDATION_REPORT_TITLE, &invalid.report) {
                Ok(()) => message.push_str(" Check the error log for details."),
                Err(e) => message.push_str(&format!(
                    " The error log could not be written ({e}), so the details follow:\n{}",
                    invalid.report
                )),
            }
        }
        return Err(message.into());
    }
    log.info("✅ Validation completed!")?;

    let csv = validator
        .to_csv(&title_to_name_mapping)
        .map_err(|e| format!("Failed to create CSV with error: {e}"))?;
    log.info(&format!("✅ CSV file created: {}", import.csv_file_name))?;

    let file_url = dkan.upload_csv(&import.csv_file_name, csv)?;
    let added = dkan.add_distribution(
        &import.dataset_id,
        NewDistribution {
            file_name: &import.csv_file_name,
            file_url: &file_url,
            data_dictionary_url: &data_dictionary.url,
        },
    )?;
    match &added.previous_csv_filename {
        Some(previous) => log.info(&format!(
            "✅ Successfully replaced CSV distribution '{}' with '{}' in dataset \"{}\" with id \"{}\"",
            previous, import.csv_file_name, added.dataset_title, import.dataset_id
        ))?,
        None => log.info(&format!(
            "✅ Successfully added CSV distribution '{}' to dataset \"{}\" with id \"{}\"",
            import.csv_file_name, added.dataset_title, import.dataset_id
        ))?,
    }

    // The CSV it replaced is no longer used
    if let Some(previous) = added.previous_csv_filename {
        dkan.delete_file(&previous)?;
        log.info(&format!(
            "🧹 Previous CSV file successfully deleted: {previous}"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
