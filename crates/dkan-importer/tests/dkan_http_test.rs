//! Tests for the DKAN HTTP calls against a local stub server - never a real DKAN instance.

use dkan_importer::model::DataDictionary;
use dkan_importer::utils::{check_upload_login, upload_distribution_csv_file};
use excel_core::reqwest::blocking::Client;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How long a test waits for the stub server to receive the request before failing.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Serve exactly one HTTP response on a local port. Returns the base URL and a receiver
/// that yields the raw request the server received.
fn serve_once(status_line: &'static str, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).unwrap();
        tx.send(String::from_utf8_lossy(&buf[..n]).to_string())
            .unwrap();
        let response = format!(
            "{status_line}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (base_url, rx)
}

#[test]
fn http_error_fetching_dictionaries_is_reported_with_its_status() {
    let (base_url, _request) = serve_once("HTTP/1.1 401 Unauthorized", "Access denied");

    let err = DataDictionary::new(&base_url, "some-id", &Client::new())
        .err()
        .expect("a 401 response must be an error");

    let message = err.to_string();
    assert!(message.contains("401"), "{message}");
    assert!(message.contains("Access denied"), "{message}");
}

#[test]
fn dictionary_request_sends_no_placeholder_authorization_header() {
    let (base_url, request) = serve_once("HTTP/1.1 200 OK", "[]");

    // An empty list means the dictionary is not found; only the request matters here
    let _ = DataDictionary::new(&base_url, "some-id", &Client::new());

    let request = request
        .recv_timeout(REQUEST_TIMEOUT)
        .expect("the server received no request");
    assert!(
        request.starts_with("GET /api/1/metastore/schemas/data-dictionary/items "),
        "{request}"
    );
    assert!(
        !request.to_lowercase().contains("authorization:"),
        "{request}"
    );
}

#[test]
fn upload_response_without_file_url_is_an_error_not_a_panic() {
    let (base_url, _request) = serve_once("HTTP/1.1 200 OK", r#"{"data":{}}"#);
    let csv = std::env::temp_dir().join(format!("dkan-importer-upload-{}.csv", std::process::id()));
    std::fs::write(&csv, "a,b\n1,2\n").unwrap();

    let result = upload_distribution_csv_file(
        &base_url,
        csv.to_str().unwrap(),
        "user",
        "password",
        &Client::new(),
    );
    let _ = std::fs::remove_file(&csv);

    let message = result
        .expect_err("a response without file_url must be an error")
        .to_string();
    assert!(message.contains("file_url"), "{message}");
}

/// What the importer module answers an authorised upload request that carries no file.
const NO_FILE_RESPONSE: &str = r#"{"error":"No file uploaded.","status":"error"}"#;

/// The start of the "Access denied" page Drupal serves for a rejected login.
const ACCESS_DENIED_PAGE: &str = "<!DOCTYPE html>\n<html><head><title>Access denied</title></head><body>You are not authorized to access this page.</body></html>";

/// A CSV file under the system temp directory, removed when dropped. `name` must be unique per test.
struct TempCsv(std::path::PathBuf);

impl TempCsv {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("dkan-importer-{name}-{}.csv", std::process::id()));
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        TempCsv(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempCsv {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn login_check_passes_when_the_importer_answers_no_file_uploaded() {
    let (base_url, request) = serve_once("HTTP/1.1 400 Bad Request", NO_FILE_RESPONSE);

    let result = check_upload_login(&base_url, "user", "password", &Client::new());

    assert!(result.is_ok(), "{result:?}");
    // Basic auth sends base64("user:password")
    let expected_authorization = "authorization: basic dxnlcjpwyxnzd29yza==";
    let request = request
        .recv_timeout(REQUEST_TIMEOUT)
        .expect("the server received no request");
    assert!(
        request.starts_with("POST /api/importer/upload "),
        "{request}"
    );
    assert!(
        request.to_lowercase().contains(expected_authorization),
        "{request}"
    );
    // No file: the check must not upload anything
    assert!(!request.to_lowercase().contains("multipart"), "{request}");
}

#[test]
fn rejected_login_is_reported_with_status_user_and_causes_but_no_page_or_password() {
    let (base_url, _request) = serve_once("HTTP/1.1 403 Forbidden", ACCESS_DENIED_PAGE);

    let message = check_upload_login(&base_url, "admin", "s3cret-pw", &Client::new())
        .expect_err("a 403 must be an error")
        .to_string();

    assert!(message.contains("rejected the login"), "{message}");
    assert!(message.contains("403"), "{message}");
    assert!(message.contains("\"admin\""), "{message}");
    assert!(message.contains("upload csv files"), "{message}");
    assert!(!message.contains("<html"), "{message}");
    assert!(!message.contains("s3cret-pw"), "{message}");
}

#[test]
fn unauthorized_login_check_is_a_rejected_login() {
    let (base_url, _request) = serve_once("HTTP/1.1 401 Unauthorized", ACCESS_DENIED_PAGE);

    let message = check_upload_login(&base_url, "admin", "password", &Client::new())
        .expect_err("a 401 must be an error")
        .to_string();

    assert!(message.contains("rejected the login"), "{message}");
    assert!(message.contains("401"), "{message}");
}

/// Only the importer's own "no file" answer proves the login; any other 400 (a proxy, a changed
/// module) must not be mistaken for it.
#[test]
fn login_check_with_any_other_bad_request_is_an_unexpected_response() {
    let (base_url, _request) = serve_once("HTTP/1.1 400 Bad Request", ACCESS_DENIED_PAGE);

    let message = check_upload_login(&base_url, "admin", "password", &Client::new())
        .expect_err("a 400 without the importer's answer must be an error")
        .to_string();

    assert!(message.contains("Unexpected response"), "{message}");
    assert!(message.contains("400"), "{message}");
    assert!(!message.contains("<html"), "{message}");
}

#[test]
fn login_check_with_a_server_error_is_an_unexpected_response() {
    let (base_url, _request) = serve_once(
        "HTTP/1.1 500 Internal Server Error",
        r#"{"error":"An error occurred.","status":"error","details":"disk full"}"#,
    );

    let message = check_upload_login(&base_url, "admin", "password", &Client::new())
        .expect_err("a 500 must be an error")
        .to_string();

    assert!(message.contains("Unexpected response"), "{message}");
    assert!(message.contains("500"), "{message}");
    assert!(message.contains("disk full"), "{message}");
}

#[test]
fn login_check_against_an_unreachable_server_names_the_url() {
    // Bind and drop a listener to get a local port nothing is listening on
    let base_url = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };

    let message = check_upload_login(&base_url, "admin", "password", &Client::new())
        .expect_err("an unreachable server must be an error")
        .to_string();

    assert!(message.contains(&base_url), "{message}");
}

#[test]
fn rejected_upload_reports_the_status_without_the_page() {
    let (base_url, _request) = serve_once("HTTP/1.1 403 Forbidden", ACCESS_DENIED_PAGE);
    let csv = TempCsv::new("upload-403");

    let message =
        upload_distribution_csv_file(&base_url, csv.path(), "user", "password", &Client::new())
            .expect_err("a 403 must be an error")
            .to_string();

    assert!(message.contains("403"), "{message}");
    assert!(!message.contains("<html"), "{message}");
}

#[test]
fn failed_upload_reports_the_importer_error_and_details() {
    let (base_url, _request) = serve_once(
        "HTTP/1.1 500 Internal Server Error",
        r#"{"error":"An error occurred while uploading the file.","status":"error","details":"disk full"}"#,
    );
    let csv = TempCsv::new("upload-500");

    let message =
        upload_distribution_csv_file(&base_url, csv.path(), "user", "password", &Client::new())
            .expect_err("a 500 must be an error")
            .to_string();

    assert!(message.contains("500"), "{message}");
    assert!(
        message.contains("An error occurred while uploading the file."),
        "{message}"
    );
    assert!(message.contains("disk full"), "{message}");
}
