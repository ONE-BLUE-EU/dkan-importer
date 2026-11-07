//! The DKAN API. Each call builds its request from its arguments, sends it with
//! [`HttpDkan::send`], the only code here doing I/O, and reads the response with a pure function.
//! So everything but the sending is tested without a network.

use crate::model::DataDictionary;
use excel_core::anyhow::{self, Context, Result};
use excel_core::reqwest::blocking::multipart::{Form, Part};
use excel_core::reqwest::blocking::{Client, Request, RequestBuilder};
use excel_core::reqwest::{Method, StatusCode, Url, header};
use excel_core::serde_json::{self, Value, json};
use serde::Deserialize;
use std::io::Read;

/// The importer module's upload endpoint
const UPLOAD_PATH: [&str; 3] = ["api", "importer", "upload"];

/// The importer module's answer to an authorised upload request that carries no file.
const NO_FILE_UPLOADED: &str = "No file uploaded.";

/// The base URL of a DKAN instance. Only `https`, as the login is sent with basic auth.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpsUrl(Url);

impl std::str::FromStr for HttpsUrl {
    type Err = anyhow::Error;

    fn from_str(url: &str) -> Result<Self> {
        let refused = || anyhow::anyhow!("The URL must be https. The provided URL is: {url}");
        let parsed = Url::parse(url).map_err(|_| refused())?;
        // An https URL with a host can always take path segments, which `endpoint` relies on
        if parsed.scheme() != "https" || parsed.host().is_none() || parsed.cannot_be_a_base() {
            return Err(refused());
        }
        Ok(HttpsUrl(parsed))
    }
}

impl std::fmt::Display for HttpsUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str().trim_end_matches('/'))
    }
}

impl HttpsUrl {
    /// The URL of `segments` under this one, each percent-encoded, so that an argument such as a
    /// file name stays one segment
    pub fn endpoint(&self, segments: &[&str]) -> Url {
        let mut url = self.0.clone();
        // Never fails: `from_str` refused the URLs that cannot be a base
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }
}

/// What DKAN answered a request
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: StatusCode,
    pub body: String,
}

/// The body of an error response from the importer module.
#[derive(Deserialize)]
struct ImporterError {
    error: String,
    details: Option<String>,
}

/// The body of the importer module's answer to an upload
#[derive(Deserialize)]
struct Uploaded {
    data: UploadedFile,
}

#[derive(Deserialize)]
struct UploadedFile {
    file_url: String,
}

/// A CSV distribution added to a dataset
#[derive(Debug, Clone, PartialEq)]
pub struct DistributionAdded {
    pub dataset_title: String,
    /// The CSV distribution it replaced, if any
    pub previous_csv_filename: Option<String>,
}

/// A CSV file uploaded, to add to a dataset as a distribution
#[derive(Debug, Clone, Copy)]
pub struct NewDistribution<'a> {
    pub file_name: &'a str,
    pub file_url: &'a str,
    /// The data dictionary describing it; the distribution it replaces is the one it describes
    pub data_dictionary_url: &'a str,
}

/// The DKAN calls of an import, logged in as one user. [`HttpDkan`] makes them over HTTP; tests
/// give the import a fake.
pub trait Dkan {
    /// Check that DKAN accepts the login for uploads, without uploading anything
    fn check_login(&self) -> Result<()>;

    /// The title of the dataset `dataset_id`
    fn dataset_title(&self, dataset_id: &str) -> Result<String>;

    /// The data dictionary `id`, once its own URL, which the distribution will name, answers
    fn data_dictionary(&self, id: &str) -> Result<DataDictionary>;

    /// Upload `csv` as the file `file_name`; returns the uploaded file's URL
    fn upload_csv(&self, file_name: &str, csv: Vec<u8>) -> Result<String>;

    /// Add `new` to the dataset `dataset_id`, replacing the CSV distribution its data dictionary
    /// describes, if any
    fn add_distribution(
        &self,
        dataset_id: &str,
        new: NewDistribution<'_>,
    ) -> Result<DistributionAdded>;

    /// Delete the uploaded file `file_name`
    fn delete_file(&self, file_name: &str) -> Result<()>;
}

/// DKAN over HTTP, logged in as one user. The password is sent with each request and never
/// written anywhere.
pub struct HttpDkan {
    base_url: HttpsUrl,
    username: String,
    password: String,
    client: Client,
}

impl HttpDkan {
    pub fn new(base_url: HttpsUrl, username: &str, password: String, client: Client) -> Self {
        HttpDkan {
            base_url,
            username: username.to_string(),
            password,
            client,
        }
    }

    /// Send `request`: the only I/O of the DKAN calls
    fn send(&self, request: Request) -> Result<Response> {
        let url = request.url().to_string();
        let response = self
            .client
            .execute(request)
            .with_context(|| format!("The request to {url} failed"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("Cannot read the response from {url}"))?;
        Ok(Response { status, body })
    }

    /// A request to `url` with the login
    fn authorised(&self, method: Method, url: Url) -> RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    fn login_request(&self) -> Result<Request> {
        Ok(self
            .authorised(Method::POST, self.base_url.endpoint(&UPLOAD_PATH))
            .build()?)
    }

    /// The data dictionaries are public: the request carries no login
    fn dictionaries_request(&self) -> Result<Request> {
        Ok(self
            .client
            .get(self.base_url.endpoint(&[
                "api",
                "1",
                "metastore",
                "schemas",
                "data-dictionary",
                "items",
            ]))
            .header(header::ACCEPT, "application/json")
            .build()?)
    }

    fn upload_request(&self, file_name: &str, csv: Vec<u8>) -> Result<Request> {
        let form = Form::new().part(
            "csv",
            Part::bytes(csv)
                .file_name(file_name.to_string())
                .mime_str("text/csv")?,
        );
        let content_type = format!("multipart/form-data; boundary={}", form.boundary());
        // The form is in memory: reading it does no I/O
        let mut body = Vec::new();
        form.into_reader()
            .read_to_end(&mut body)
            .context("Cannot build the upload request")?;
        Ok(self
            .authorised(Method::POST, self.base_url.endpoint(&UPLOAD_PATH))
            .header(header::CONTENT_TYPE, content_type)
            .body(body)
            .build()?)
    }

    fn dataset_request(&self, method: Method, dataset_id: &str) -> RequestBuilder {
        self.authorised(
            method,
            self.base_url.endpoint(&[
                "api",
                "1",
                "metastore",
                "schemas",
                "dataset",
                "items",
                dataset_id,
            ]),
        )
    }

    /// The DELETE method is not supported for this endpoint, so it is a POST
    fn delete_request(&self, file_name: &str) -> Result<Request> {
        Ok(self
            .authorised(
                Method::POST,
                self.base_url
                    .endpoint(&["api", "importer", "delete", file_name]),
            )
            .build()?)
    }
}

impl Dkan for HttpDkan {
    /// The importer answers an upload request without a file with "No file uploaded." only once
    /// the login and the "upload csv files" permission have been accepted.
    fn check_login(&self) -> Result<()> {
        let response = self.send(self.login_request()?)?;
        login_checked(&self.base_url, &self.username, &response)
    }

    fn dataset_title(&self, dataset_id: &str) -> Result<String> {
        let response = self.send(self.dataset_request(Method::GET, dataset_id).build()?)?;
        dataset_title(dataset_id, &response)
    }

    fn data_dictionary(&self, id: &str) -> Result<DataDictionary> {
        let request = self.dictionaries_request()?;
        let url = request.url().to_string();
        let response = self.send(request)?;
        let dictionary =
            DataDictionary::from_items(&self.base_url, id, dictionaries(&url, &response)?)?;
        let response = self.send(self.client.get(&dictionary.url).build()?)?;
        dictionary_found(id, &dictionary.url, &response)?;
        Ok(dictionary)
    }

    fn upload_csv(&self, file_name: &str, csv: Vec<u8>) -> Result<String> {
        let response = self.send(self.upload_request(file_name, csv)?)?;
        uploaded_file_url(&response)
    }

    fn add_distribution(
        &self,
        dataset_id: &str,
        new: NewDistribution<'_>,
    ) -> Result<DistributionAdded> {
        let response = self.send(self.dataset_request(Method::GET, dataset_id).build()?)?;
        let (dataset, added) = dataset_with_distribution(dataset_id, &response, new)?;
        let response = self.send(
            self.dataset_request(Method::PATCH, dataset_id)
                .json(&dataset)
                .build()?,
        )?;
        distribution_saved(dataset_id, &added, &response)?;
        Ok(added)
    }

    fn delete_file(&self, file_name: &str) -> Result<()> {
        let response = self.send(self.delete_request(file_name)?)?;
        file_deleted(file_name, &response)
    }
}

/// `HTTP <status>`, followed by the importer's error and details when the body has them. Any other
/// body, such as a Drupal HTML page, is left out.
fn describe_failure(status: StatusCode, body: &str) -> String {
    match serde_json::from_str::<ImporterError>(body) {
        Ok(ImporterError {
            error,
            details: Some(details),
        }) => format!("HTTP {status}: {error} ({details})"),
        Ok(ImporterError {
            error,
            details: None,
        }) => format!("HTTP {status}: {error}"),
        Err(_) => format!("HTTP {status}"),
    }
}

/// Whether the answer to the login check of `username` at `base_url` accepts the login
fn login_checked(base_url: &HttpsUrl, username: &str, response: &Response) -> Result<()> {
    let Response { status, body } = response;
    match *status {
        // Drupal does not say which: a wrong password and a missing permission look the same
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(anyhow::anyhow!(
            "DKAN at {base_url} rejected the login for user \"{username}\" (HTTP {status}). Check the username and password, that the account is active and not locked out, and that it has the \"upload csv files\" permission."
        )),
        StatusCode::BAD_REQUEST
            if serde_json::from_str::<ImporterError>(body)
                .is_ok_and(|e| e.error == NO_FILE_UPLOADED) =>
        {
            Ok(())
        }
        _ => Err(anyhow::anyhow!(
            "Unexpected response checking the login at {}: {}",
            base_url.endpoint(&UPLOAD_PATH),
            describe_failure(*status, body)
        )),
    }
}

/// The body listing the data dictionaries, fetched from `url`
fn dictionaries<'a>(url: &str, response: &'a Response) -> Result<&'a str> {
    let Response { status, body } = response;
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "Failed to fetch the data dictionaries from {url}: HTTP {status}: {body}"
        ));
    }
    Ok(body)
}

/// Whether the data dictionary `id` answers at its own `url`
fn dictionary_found(id: &str, url: &str, response: &Response) -> Result<()> {
    if !response.status.is_success() {
        return Err(anyhow::anyhow!(
            "Failed to validate the existence of the data dictionary {id}. \
            Please check if the data dictionary exists and is accessible at {url}"
        ));
    }
    Ok(())
}

/// The uploaded file's URL, from the importer's answer to the upload
fn uploaded_file_url(response: &Response) -> Result<String> {
    let Response { status, body } = response;
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "Custom importer upload failed: {}",
            describe_failure(*status, body)
        ));
    }
    let uploaded: Uploaded = serde_json::from_str(body)
        .map_err(|_| anyhow::anyhow!("The upload response has no data.file_url: {body}"))?;
    Ok(uploaded.data.file_url)
}

/// The dataset `dataset_id`, read from `response`, with its title
fn dataset(dataset_id: &str, response: &Response) -> Result<(Value, String)> {
    let Response { status, body } = response;
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "Failed to get dataset {dataset_id}: {body}"
        ));
    }
    let dataset: Value = serde_json::from_str(body)
        .with_context(|| format!("The dataset {dataset_id} is not JSON"))?;
    let title = dataset
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Dataset title not found"))?
        .to_string();
    Ok((dataset, title))
}

/// The title of the dataset `dataset_id`, read from `response`
fn dataset_title(dataset_id: &str, response: &Response) -> Result<String> {
    dataset(dataset_id, response).map(|(_, title)| title)
}

/// The dataset `dataset_id`, read from `response`, with `new` in place of the CSV distribution
/// its data dictionary describes, if any; and what that adds
fn dataset_with_distribution(
    dataset_id: &str,
    response: &Response,
    new: NewDistribution<'_>,
) -> Result<(Value, DistributionAdded)> {
    let (mut dataset, dataset_title) = dataset(dataset_id, response)?;

    let mut previous_csv_filename = None;
    let mut distributions = Vec::new();
    let existing = dataset
        .get("distribution")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for distribution in existing {
        let replaced = distribution.get("describedBy").and_then(Value::as_str)
            == Some(new.data_dictionary_url);
        if !replaced {
            distributions.push(distribution);
            continue;
        }
        // Named by its title, or else by the last segment of its download URL
        previous_csv_filename = distribution
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| {
                distribution
                    .get("downloadURL")
                    .and_then(Value::as_str)
                    .and_then(|url| url.split('/').next_back())
            })
            .map(String::from)
            .or(previous_csv_filename);
    }
    distributions.push(json!({
        "title": new.file_name,
        "description": format!("Data file: {}", new.file_name),
        "format": "csv",
        "mediaType": "text/csv",
        "downloadURL": new.file_url,
        "describedBy": new.data_dictionary_url,
        "describedByType": "application/vnd.tableschema+json",
    }));
    if let Some(object) = dataset.as_object_mut() {
        object.insert("distribution".to_string(), Value::Array(distributions));
    }
    Ok((
        dataset,
        DistributionAdded {
            dataset_title,
            previous_csv_filename,
        },
    ))
}

/// Whether DKAN saved the dataset `dataset_id` with its new distribution
fn distribution_saved(
    dataset_id: &str,
    added: &DistributionAdded,
    response: &Response,
) -> Result<()> {
    if !response.status.is_success() {
        return Err(anyhow::anyhow!(
            "Failed to add CSV distribution to dataset \"{}\" with id \"{}\" with error: {}",
            added.dataset_title,
            dataset_id,
            response.body
        ));
    }
    Ok(())
}

/// Whether DKAN deleted the uploaded file `file_name`
fn file_deleted(file_name: &str, response: &Response) -> Result<()> {
    if !response.status.is_success() {
        return Err(anyhow::anyhow!(
            "Failed to delete file {file_name}: {}",
            response.body
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
