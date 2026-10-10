//! Tests for the DKAN HTTP calls against a local stub server - never a real DKAN instance.

use dkan_importer::model::DataDictionary;
use dkan_importer::utils::upload_distribution_csv_file;
use excel_core::reqwest::blocking::Client;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

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

    let request = request.recv().unwrap();
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
