use importer_lib::anyhow;
use importer_lib::reqwest::blocking::multipart::{Form, Part};
use importer_lib::reqwest::blocking::Client;
use importer_lib::serde_json;
use importer_lib::utils::{get_local_datetime_with_format, normalize_string};

pub fn generate_unique_filename(dataset_id: &str, excel_sheet_name: &str) -> String {
    let timestamp = get_local_datetime_with_format("%Y-%m-%d_%H-%M-%S");
    let excel_sheet_name = normalize_string(excel_sheet_name);
    let filename = format!("{excel_sheet_name}_{timestamp}_{dataset_id}.csv");
    return filename.to_lowercase();
}

// Function to upload CSV to custom importer endpoint
pub fn upload_distribution_csv_file(
    url: &str,
    csv_path: &str,
    username: &str,
    password: &str,
    client: &Client,
) -> Result<String, anyhow::Error> {
    let csv_content = std::fs::read(csv_path)?;
    let filename = std::path::Path::new(csv_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.csv");

    // Create multipart form with the CSV file
    let form = Form::new().part(
        "csv",
        Part::bytes(csv_content)
            .file_name(filename.to_string())
            .mime_str("text/csv")?,
    );

    let upload_url = format!("{}/api/importer/upload", url);

    let response = client
        .post(&upload_url)
        .basic_auth(username, Some(password))
        .multipart(form)
        .send()?;

    let status = response.status();

    if status.is_success() {
        let response_text = response.text()?;

        // Extract the file_url from the response and return it
        let response_json: serde_json::Value = serde_json::from_str(&response_text)?;
        let file_url = response_json["data"]["file_url"]
            .as_str()
            .expect("File URL not found")
            .to_string();
        Ok(file_url)
    } else {
        let error_text = response.text()?;
        Err(anyhow::anyhow!(
            "Custom importer upload failed: {}",
            error_text
        ))
    }
}

pub fn dataset_add_distribution(
    url: &str,
    dataset_id: &str,
    file_name: &str,
    file_url: &str,
    data_dictionary_url: &str,
    username: &str,
    password: &str,
    client: &Client,
) -> Result<Option<String>, anyhow::Error> {
    // Step 1: Get the current dataset to ensure it exists and get its current state
    let endpoint_url = format!("{url}/api/1/metastore/schemas/dataset/items/{dataset_id}");
    let get_response = client
        .get(&endpoint_url)
        .basic_auth(username, Some(password))
        .send()?;

    if !get_response.status().is_success() {
        let error_text = get_response.text()?;
        return Err(anyhow::anyhow!(
            "Failed to get dataset {dataset_id}: {error_text}"
        ));
    }

    let mut dataset: serde_json::Value = get_response.json()?;
    let dataset_title = dataset["title"]
        .as_str()
        .ok_or(anyhow::anyhow!("Dataset title not found"))?
        .to_string();

    // Step 2: Create the new CSV distribution
    let new_distribution = serde_json::json!({
        "title": file_name,
        "description": format!("Data file: {}", file_name),
        "format": "csv",
        "mediaType": "text/csv",
        "downloadURL": file_url,
        "describedBy": data_dictionary_url,
        "describedByType": "application/vnd.tableschema+json",
    });

    // Step 3: Get existing distributions and find the one to replace
    let existing_distributions = dataset["distribution"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    // Find and extract the filename of the distribution being replaced
    let mut previous_csv_filename: Option<String> = None;

    // Separate distributions: keep non-matching ones, extract filename from matching ones
    let mut filtered_distributions = Vec::new();

    for dist in existing_distributions {
        let matches_data_dictionary = dist
            .get("describedBy")
            .and_then(|described_by| described_by.as_str())
            .map(|url| url == data_dictionary_url)
            .unwrap_or(false);

        if matches_data_dictionary {
            // Extract the filename from the distribution being replaced
            if let Some(title) = dist.get("title").and_then(|t| t.as_str()) {
                previous_csv_filename = Some(title.to_string());
            } else if let Some(download_url) = dist.get("downloadURL").and_then(|u| u.as_str()) {
                // Try to extract filename from downloadURL if title is not available
                if let Some(filename) = download_url.split('/').next_back() {
                    previous_csv_filename = Some(filename.to_string());
                }
            }
            // Don't add this distribution to filtered_distributions (it gets replaced)
        } else {
            // Keep this distribution (it doesn't match the data dictionary)
            filtered_distributions.push(dist);
        }
    }

    // Add the new distribution
    filtered_distributions.push(new_distribution);

    // Step 4: Update the dataset with the modified distributions array
    dataset["distribution"] = serde_json::Value::Array(filtered_distributions);

    // Step 5: Update the dataset with the new distribution
    let patch_response = client
        .patch(&endpoint_url)
        .basic_auth(username, Some(password))
        .header("Content-Type", "application/json")
        .json(&dataset)
        .send()?;

    if patch_response.status().is_success() {
        if let Some(ref prev_filename) = previous_csv_filename {
            println!("✅ Successfully replaced CSV distribution '{}' with '{}' in dataset \"{}\" with id \"{}\"",
                prev_filename, file_name, dataset_title, dataset_id);
        } else {
            println!(
                "✅ Successfully added CSV distribution '{}' to dataset \"{}\" with id \"{}\"",
                file_name, dataset_title, dataset_id
            );
        }
        Ok(previous_csv_filename)
    } else {
        let error_text = patch_response.text()?;
        Err(anyhow::anyhow!(
            "Failed to add CSV distribution to dataset \"{}\" with id \"{}\" with error: {}",
            dataset_title,
            dataset_id,
            error_text
        ))
    }
}

pub fn delete_remote_file(
    url: &str,
    file_name: &str,
    username: &str,
    password: &str,
    client: &Client,
) -> Result<(), anyhow::Error> {
    let endpoint_url = format!("{url}/api/importer/delete/{file_name}");
    let response = client
        // The DELETE method is not supported for this endpoint, so we use POST instead
        .post(&endpoint_url)
        .basic_auth(username, Some(password))
        .send()?;

    if !response.status().is_success() {
        let error_text = response.text()?;
        return Err(anyhow::anyhow!(
            "Failed to delete file {file_name}: {error_text}"
        ));
    }
    println!("🧹 Previous CSV file successfully deleted: {file_name}");
    return Ok(());
}
