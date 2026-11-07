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

/// Why the first line of a run stopped before reaching DKAN has no dataset name
const DKAN_NOT_REACHED: &str = "the run stopped before reaching DKAN";

/// The dataset's name, for the first line of the run log
enum DatasetName {
    Read(String),
    /// Why not
    NotRead(String),
}

/// The first line of the run log: the file, and the dataset with its name. One line, whatever
/// the reason the name was not read.
fn first_line(import: &Import, name: &DatasetName) -> String {
    let name = match name {
        DatasetName::Read(title) => format!("\"{title}\""),
        DatasetName::NotRead(why) => format!(
            "name not read: {}",
            why.split_whitespace().collect::<Vec<_>>().join(" ")
        ),
    };
    format!(
        "Importing \"{}\" into dataset {} ({name})",
        import.excel_file, import.dataset_id
    )
}

/// The steps before the first line of the run log can name the dataset; they log nothing
struct Reached<D> {
    dkan: D,
    workbook: Vec<u8>,
    dataset_name: DatasetName,
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
        Ok(title) => DatasetName::Read(title),
        Err(e) => DatasetName::NotRead(e.to_string()),
    };
    Ok(Reached {
        dkan,
        workbook,
        dataset_name,
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
    let not_reached = DatasetName::NotRead(DKAN_NOT_REACHED.to_string());
    let dataset_name = reached
        .as_ref()
        .map_or(&not_reached, |reached| &reached.dataset_name);
    log.note(&first_line(import, dataset_name))?;
    log.note(&format!("Excel file: {}", import.excel_file))?;
    log.note(&format!("Sheet: {}", import.sheet_name))?;
    log.note(&format!("Dataset: {}", import.dataset_id))?;
    log.note(&format!("Data dictionary: {}", import.data_dictionary_id))?;
    log.note(&format!("DKAN: {}", import.base_url))?;

    let Reached { dkan, workbook, .. } = reached?;
    log.info(&format!(
        "✅ Login accepted for user \"{}\"",
        import.username
    ))?;
    let data_dictionary = dkan.data_dictionary(&import.data_dictionary_id)?;
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

    fn import() -> Import {
        Import {
            excel_file: "samples.xlsx".to_string(),
            sheet_name: "Sheet1".to_string(),
            data_dictionary_id: "dd-1".to_string(),
            dataset_id: "ds-1".to_string(),
            base_url: "https://dkan.example".to_string(),
            username: "admin".to_string(),
            csv_file_name: "samples.csv".to_string(),
            unpivot: None,
            extra_columns_to: None,
        }
    }

    /// A reason such as an HTML error page keeps the first line one line
    #[test]
    fn the_first_line_stays_one_line_whatever_the_reason_the_name_was_not_read() {
        let name = DatasetName::NotRead(
            "Failed to get dataset ds-1: <html>\n  <body>Not found</body>\n</html>".to_string(),
        );

        let expected = "Importing \"samples.xlsx\" into dataset ds-1 (name not read: Failed to get dataset ds-1: <html> <body>Not found</body> </html>)";
        assert_eq!(first_line(&import(), &name), expected);
    }
}
