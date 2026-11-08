// API for getting the JSON Schema (Data dictionary): /api/1/metastore/schemas/{schema_id}/items

// reset; cargo run -- --url https://dkan.ddev.site --excel-file a --schema-name "Samples Dictionary"
// reset; cargo run -- --url https://dkan.ddev.site --excel-file ./data/Sample_Collection_North_Adriatic_26Feb2025.xlsx --sheet-name Sample --schema-name "Samples Dictionary"

use clap::Parser;
use dkan_importer::{
    dkan::{HttpDkan, HttpsUrl},
    error_log::append_error_to_log,
    import::{Environment, Import, SheetNames, run},
    run_log::RunLog,
    utils::RunFiles,
};
use excel_core::anyhow;
use excel_core::reqwest::blocking::Client;
use excel_core::{
    DEFAULT_MARKER_LIMITS, DEFAULT_VALUES_MARKERS, MarkerLimit, Unpivot, parse_values_marker,
};
use rpassword::prompt_password;
use std::io;
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "dkan-importer")]
#[command(about = "A tool to validate Excel files against JSON schemas")]
#[command(version)]
struct Args {
    /// URL to fetch the JSON schema from, and to where the data will be uploaded
    #[arg(short, long)]
    base_url: String,

    /// Absolute path to the Excel file to validate (the file that will be validated against the JSON schema)
    #[arg(short, long)]
    excel_file: String,

    /// The UUID of the DKAN data dictionary that will be used to validate the Excel file
    #[arg(long)]
    data_dictionary_id: String,

    /// The sheet to import (Sheet1 if not given). Give it more than once to merge several
    /// sheets with the same columns into one CSV file, in the order given; each is read and
    /// validated on its own, and nothing is uploaded unless every one is accepted.
    ///
    /// Example, merging the analytical results of two matrices:
    ///   dkan-importer ... --unpivot --headers-column "Sample Tag" \
    ///     --sheet-name "Analytical results (Air)" \
    ///     --sheet-name "Analytical results (Sediment)"
    #[arg(long, default_value = "Sheet1", verbatim_doc_comment)]
    sheet_name: Vec<String>,

    /// The username for the remote API authentication.
    #[arg(long)]
    username: String,

    /// The password for the remote API authentication. If not specified, the password will be required during runtime.
    #[arg(long)]
    password: Option<String>,

    /// The UUID of the existing DKAN dataset to add the CSV file as a distribution
    #[arg(long)]
    dataset_id: String,

    /// Unpivot the sheet: turn each sample column into one row per cell.
    ///
    /// How the sheet is split: the leading columns whose headers are data
    /// dictionary fields are copied to every row. Every column after them is a
    /// sample column. Each non-blank sample cell becomes a row: the column's
    /// header goes to --headers-column and the cell to --values-column. Both
    /// must be data dictionary fields that are not columns in the sheet.
    ///
    /// The sheet holds analytical results: it must have an "Identification
    /// Level" column before the sample columns. In every row with results it
    /// must hold a number, uploaded as written, or be blank, uploaded as 2.
    /// Anything else, text included, stops the run, listing the rows to
    /// correct.
    ///
    /// Example:
    ///   dkan-importer ... --sheet-name "Analytical results" \
    ///     --unpivot --headers-column "Sample Tag" --values-column "Concentration"
    ///
    ///   Substance name | ST01_Surface | ST02_Surface
    ///   Okadaic acid   | 0.88         | 0.40
    ///
    ///   becomes
    ///
    ///   Substance name | Sample Tag   | Concentration
    ///   Okadaic acid   | ST01_Surface | 0.88
    ///   Okadaic acid   | ST02_Surface | 0.40
    #[arg(
        long,
        requires = "headers_column",
        help_heading = "Unpivot",
        verbatim_doc_comment
    )]
    unpivot: bool,

    /// The data dictionary field that receives each sample column's header.
    /// Only with --unpivot.
    #[arg(
        long,
        value_name = "FIELD",
        requires = "unpivot",
        help_heading = "Unpivot",
        verbatim_doc_comment
    )]
    headers_column: Option<String>,

    /// The data dictionary field that receives each sample cell's value.
    /// Only with --unpivot.
    #[arg(
        long,
        value_name = "FIELD",
        default_value = "Concentration",
        requires = "unpivot",
        help_heading = "Unpivot",
        verbatim_doc_comment
    )]
    values_column: String,

    /// The text a sample cell may hold instead of a number, when the values
    /// column is a text field. Only with --unpivot.
    ///
    /// Analytical results do not always have a number: a substance can be
    /// below the limit of detection (<LOD) or of quantification (<LOQ), or
    /// detected in a Peak Area row (X). Keeping these as written needs a values column
    /// that takes text, but a text field accepts anything, so a typo such as
    /// "<LDO" or "n.d." would be uploaded as data. With this option, each
    /// non-blank sample cell must be a number or one of these markers (ignoring
    /// case and surrounding spaces; markers are written uppercase, so <lod is
    /// uploaded as <LOD). Anything else is a validation error, listed with its
    /// row in the run's error log.
    ///
    /// Giving markers replaces the default ones; list all you need after one
    /// flag. A sample cell holding NS (no sample), or ND (treated as NS), gives
    /// no row, so neither can be a marker. <LQ and NQ are refused, even when
    /// given here: they map to <LOQ, so change them in the source file, for
    /// consistency. If the values column is a number field, the markers are not
    /// used: the field already refuses all text, including these markers.
    ///
    /// Example:
    ///   dkan-importer ... --unpivot --headers-column "Sample Tag" \
    ///     --values-markers "<LOD" "<LOQ" "<MDL"
    #[arg(
        long,
        value_name = "MARKER",
        num_args = 1..,
        default_values = DEFAULT_VALUES_MARKERS,
        value_parser = parse_values_marker,
        requires = "unpivot",
        help_heading = "Unpivot",
        verbatim_doc_comment
    )]
    values_markers: Vec<String>,

    /// The markers that may be followed by their limit, as MARKER=FIELD.
    ///
    /// Some sheets write the limit after the marker, as in "<LOQ:0.25". The
    /// limit is already in its own column, so when the number after the
    /// marker equals FIELD in the same row, the cell is uploaded as the marker
    /// alone ("<LOQ"), the same as in sheets without the number. A different
    /// number, or a row with FIELD blank, is a validation error: no number is
    /// dropped that the row does not keep.
    ///
    /// Giving limits replaces the default ones. A limit for a marker that is
    /// not one of --values-markers does not apply. FIELD must be a data
    /// dictionary field.
    ///
    /// Example:
    ///   dkan-importer ... --unpivot --headers-column "Sample Tag" \
    ///     --marker-limits "<LOQ=Limit of quantification"
    #[arg(
        long,
        value_name = "MARKER=FIELD",
        num_args = 1..,
        default_values = DEFAULT_MARKER_LIMITS,
        value_parser = clap::value_parser!(MarkerLimit),
        requires = "unpivot",
        help_heading = "Unpivot",
        verbatim_doc_comment
    )]
    marker_limits: Vec<MarkerLimit>,

    /// Move the sheet's columns that are not data dictionary fields into FIELD
    /// as JSON, instead of refusing them. [default when given alone: Remark 1]
    ///
    /// FIELD must be a data dictionary field that takes text. For each row with
    /// data in those columns, FIELD gets a JSON object of their non-blank cells,
    /// keyed by header in sheet order. Text already in FIELD is kept in the
    /// object under FIELD's own header, first. Rows without such data keep FIELD
    /// as it is. Not with --unpivot.
    ///
    /// Example:
    ///   dkan-importer ... --sheet-name "Sample" --extra-columns-to "Remark 3"
    ///
    ///   Sample Tag | Remark 3    | Slice num | DNA (Yes/No)
    ///   ST01-a     |             | 1         | yes
    ///   ST01-b     | Super dirty | 2         |
    ///
    ///   becomes
    ///
    ///   Sample Tag | Remark 3
    ///   ST01-a     | {"Slice num":1,"DNA (Yes/No)":"yes"}
    ///   ST01-b     | {"Remark 3":"Super dirty","Slice num":2}
    #[arg(
        long,
        value_name = "FIELD",
        num_args = 0..=1,
        default_missing_value = "Remark 1",
        conflicts_with = "unpivot",
        verbatim_doc_comment
    )]
    extra_columns_to: Option<String>,
}

fn main() -> ExitCode {
    let arguments = Args::parse();
    let sheet_names = match SheetNames::new(arguments.sheet_name) {
        Ok(sheet_names) => sheet_names,
        Err(e) => {
            eprintln!("❌ {e}");
            return ExitCode::FAILURE;
        }
    };
    let files = RunFiles::now(&arguments.dataset_id, &sheet_names);
    let unpivot = match (arguments.unpivot, arguments.headers_column) {
        (true, Some(headers_column)) => Some(Unpivot {
            headers_column,
            values_column: arguments.values_column,
            values_markers: arguments.values_markers,
            marker_limits: arguments.marker_limits,
        }),
        // clap makes --unpivot require --headers-column
        (true, None) => {
            eprintln!("❌ --unpivot requires --headers-column");
            return ExitCode::FAILURE;
        }
        (false, _) => None,
    };
    let import = Import {
        excel_file: arguments.excel_file,
        sheet_names,
        data_dictionary_id: arguments.data_dictionary_id,
        dataset_id: arguments.dataset_id,
        base_url: arguments.base_url,
        username: arguments.username,
        csv_file_name: files.csv,
        unpivot,
        extra_columns_to: arguments.extra_columns_to,
    };
    let mut environment = Terminal {
        password: arguments.password,
        errors_log: files.errors,
    };

    let log_path = files.log;
    let mut log = match RunLog::create(&log_path) {
        Ok(log) => log,
        Err(e) => {
            eprintln!("❌ Cannot write the run log \"{log_path}\": {e}");
            return ExitCode::FAILURE;
        }
    };
    // The run prints its own error, and last what became of the CSV file and of the upload
    match run(&import, &mut environment, &mut log) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(log_error) => {
            eprintln!("❌ Cannot write the run log \"{log_path}\": {log_error}");
            ExitCode::FAILURE
        }
    }
}

/// The real environment of an import: the file system, DKAN over HTTP, and the terminal to ask
/// for the password
struct Terminal {
    /// The password given with --password, if any; asked for otherwise
    password: Option<String>,
    /// The run's error log; see [`RunFiles`]
    errors_log: String,
}

impl Environment for Terminal {
    type Dkan = HttpDkan;

    fn read_excel(&mut self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn connect(&mut self, base_url: HttpsUrl, username: &str) -> anyhow::Result<HttpDkan> {
        let password = match self.password.take() {
            Some(password) => password,
            None => prompt_password("Password: ")
                .map_err(|e| anyhow::anyhow!("Failed to read the password: {e}"))?,
        };
        Ok(HttpDkan::new(base_url, username, password, Client::new()))
    }

    fn save_csv(&mut self, file_name: &str, csv: &[u8]) -> io::Result<()> {
        std::fs::write(file_name, csv)
    }

    fn append_error_log(&mut self, title: &str, details: &str) -> io::Result<()> {
        append_error_to_log(Path::new(&self.errors_log), title, details)
    }
}
