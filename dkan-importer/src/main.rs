// API for getting the JSON Schema (Data dictionary): /api/1/metastore/schemas/{schema_id}/items

// reset; cargo run -- --url https://dkan.ddev.site --excel-file a --schema-name "Samples Dictionary"
// reset; cargo run -- --url https://dkan.ddev.site --excel-file ./data/Sample_Collection_North_Adriatic_26Feb2025.xlsx --sheet-name Sample --schema-name "Samples Dictionary"

use clap::Parser;
use dkan_importer::{
    model::DataDictionary,
    utils::{
        dataset_add_distribution, delete_remote_file, generate_unique_filename,
        upload_distribution_csv_file,
    },
};
use importer_lib::reqwest::blocking::Client;
use importer_lib::ExcelValidatorBuilder;
use rpassword::prompt_password;

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

    /// Optional sheet name to validate (if not specified, validates Sheet1)
    #[arg(long, default_value = "Sheet1")]
    sheet_name: String,

    /// The username for the remote API authentication.
    #[arg(long)]
    username: String,

    /// The password for the remote API authentication. If not specified, the password will be required during runtime.
    #[arg(long)]
    password: Option<String>,

    /// The UUID of the existing DKAN dataset to add the CSV file as a distribution
    #[arg(long)]
    dataset_id: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Args::parse();

    // Validate the url before asking for anything. It must be https because we are using basic auth.
    if !arguments.base_url.starts_with("https://") {
        return Err(format!(
            "The URL must be https. The provided URL is: {}",
            arguments.base_url
        )
        .into());
    }

    let password = match arguments.password {
        Some(password) => password,
        None => prompt_password("Password: ")
            .map_err(|e| format!("Failed to read the password: {e}"))?,
    };
    let client = Client::new();
    let data_dictionary =
        DataDictionary::new(&arguments.base_url, &arguments.data_dictionary_id, &client)?;
    let json_schema = data_dictionary.to_json_schema()?;
    let title_to_name_mapping =
        DataDictionary::create_title_to_name_mapping(&data_dictionary.fields)?;
    let mut validator =
        ExcelValidatorBuilder::new(&arguments.excel_file, &arguments.sheet_name, json_schema)
            .build()?;
    if let Err(e) = validator.validate_excel() {
        // The error says whether the details are in the error log or in the message itself
        eprintln!("❌ {e}");
        std::process::exit(1);
    }
    println!("✅ Validation completed!");

    let csv_filename = generate_unique_filename(&arguments.dataset_id, &arguments.sheet_name);
    // Create a csv since the validation is successful. Use schema-aware parsing for proper date formatting.
    validator
        .export_to_csv(&csv_filename, title_to_name_mapping)
        .map_err(|e| format!("❌ Failed to create CSV with error: {e}"))?;
    println!("✅ CSV file created: {csv_filename}");

    let file_url = upload_distribution_csv_file(
        &arguments.base_url,
        &csv_filename,
        &arguments.username,
        &password,
        &client,
    )?;

    let optional_previous_csv_filename = dataset_add_distribution(
        &arguments.base_url,
        &arguments.dataset_id,
        &csv_filename,
        &file_url,
        &data_dictionary.url,
        &arguments.username,
        &password,
        &client,
    )?;

    // Clean up previous CSV file if one was replaced
    if let Some(previous_csv_filename) = optional_previous_csv_filename {
        delete_remote_file(
            &arguments.base_url,
            &previous_csv_filename,
            &arguments.username,
            &password,
            &client,
        )?;
    }

    // Also delete the CSV file from the local filesystem
    std::fs::remove_file(&csv_filename)?;

    Ok(())
}
