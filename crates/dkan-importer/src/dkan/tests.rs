//! The DKAN calls without a network: each request as built, and each answer as read.

use super::*;

/// What the importer module answers an authorised upload request that carries no file.
const NO_FILE_RESPONSE: &str = r#"{"error":"No file uploaded.","status":"error"}"#;

/// The start of the "Access denied" page Drupal serves for a rejected login.
const ACCESS_DENIED_PAGE: &str = "<!DOCTYPE html>\n<html><head><title>Access denied</title></head><body>You are not authorized to access this page.</body></html>";

/// Basic auth of "user" with "password": base64("user:password")
const BASIC_AUTH: &str = "Basic dXNlcjpwYXNzd29yZA==";

const DICTIONARY_URL: &str =
    "https://dkan.example/api/1/metastore/schemas/data-dictionary/items/dd-1";

fn base_url() -> HttpsUrl {
    "https://dkan.example".parse().unwrap()
}

fn dkan() -> HttpDkan {
    HttpDkan::new(base_url(), "user", "password".to_string(), Client::new())
}

fn response(status: u16, body: &str) -> Response {
    Response {
        status: StatusCode::from_u16(status).unwrap(),
        body: body.to_string(),
    }
}

fn header(request: &Request, name: header::HeaderName) -> Option<&str> {
    request.headers().get(name).and_then(|v| v.to_str().ok())
}

fn body(request: &Request) -> &[u8] {
    request
        .body()
        .and_then(|body| body.as_bytes())
        .expect("the request must have a body in memory")
}

fn new_distribution() -> NewDistribution<'static> {
    NewDistribution {
        file_name: "new.csv",
        file_url: "https://dkan.example/files/new.csv",
        data_dictionary_url: DICTIONARY_URL,
    }
}

// The base URL

#[test]
fn an_https_url_is_taken_without_its_trailing_slash() {
    let url: HttpsUrl = "https://dkan.example/".parse().unwrap();

    assert_eq!(url.to_string(), "https://dkan.example");
}

#[test]
fn a_url_that_is_not_https_is_refused_before_any_request() {
    for url in [
        "http://dkan.example",
        "dkan.example",
        "ftp://dkan.example",
        "",
    ] {
        let message = url
            .parse::<HttpsUrl>()
            .expect_err("only https is accepted")
            .to_string();

        let expected = format!("The URL must be https. The provided URL is: {url}");
        assert_eq!(message, expected);
    }
}

#[test]
fn endpoints_keep_the_base_path_and_encode_each_segment() {
    let url: HttpsUrl = "https://dkan.example/dkan/".parse().unwrap();

    let endpoint = url.endpoint(&["api", "importer", "delete", "a b/../c?.csv"]);

    let expected = "https://dkan.example/dkan/api/importer/delete/a%20b%2F..%2Fc%3F.csv";
    assert_eq!(endpoint.as_str(), expected);
}

// The login check

#[test]
fn the_login_check_posts_the_login_to_the_upload_endpoint_without_a_file() {
    let request = dkan().login_request().unwrap();

    assert_eq!(request.method(), Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://dkan.example/api/importer/upload"
    );
    assert_eq!(header(&request, header::AUTHORIZATION), Some(BASIC_AUTH));
    assert!(request.body().is_none());
}

#[test]
fn the_login_is_accepted_when_the_importer_answers_no_file_uploaded() {
    let result = login_checked(&base_url(), "user", &response(400, NO_FILE_RESPONSE));

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn a_forbidden_login_is_rejected_with_status_user_and_causes_but_no_page() {
    let message = login_checked(&base_url(), "admin", &response(403, ACCESS_DENIED_PAGE))
        .expect_err("a 403 must be an error")
        .to_string();

    let expected = "DKAN at https://dkan.example rejected the login for user \"admin\" (HTTP 403 Forbidden). Check the username and password, that the account is active and not locked out, and that it has the \"upload csv files\" permission.";
    assert_eq!(message, expected);
}

#[test]
fn an_unauthorized_login_is_rejected() {
    let message = login_checked(&base_url(), "admin", &response(401, ACCESS_DENIED_PAGE))
        .expect_err("a 401 must be an error")
        .to_string();

    assert!(message.contains("rejected the login"), "{message}");
    assert!(message.contains("HTTP 401"), "{message}");
}

/// Only the importer's own "no file" answer proves the login; any other 400 (a proxy, a changed
/// module) must not be mistaken for it.
#[test]
fn any_other_bad_request_is_an_unexpected_answer_to_the_login_check() {
    let message = login_checked(&base_url(), "admin", &response(400, ACCESS_DENIED_PAGE))
        .expect_err("a 400 without the importer's answer must be an error")
        .to_string();

    let expected = "Unexpected response checking the login at https://dkan.example/api/importer/upload: HTTP 400 Bad Request";
    assert_eq!(message, expected);
}

#[test]
fn a_server_error_answering_the_login_check_gives_the_importers_error_and_details() {
    let message = login_checked(
        &base_url(),
        "admin",
        &response(
            500,
            r#"{"error":"An error occurred.","status":"error","details":"disk full"}"#,
        ),
    )
    .expect_err("a 500 must be an error")
    .to_string();

    let expected = "Unexpected response checking the login at https://dkan.example/api/importer/upload: HTTP 500 Internal Server Error: An error occurred. (disk full)";
    assert_eq!(message, expected);
}

/// The one test that sends: to a local port nothing listens on, so nothing leaves the machine.
#[test]
fn an_unreachable_dkan_is_an_error_naming_the_url() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let base_url: HttpsUrl = format!("https://127.0.0.1:{port}").parse().unwrap();
    let dkan = HttpDkan::new(base_url, "user", "password".to_string(), Client::new());

    let message = format!(
        "{:#}",
        dkan.check_login().expect_err("nothing listens on the port")
    );

    let expected = format!("The request to https://127.0.0.1:{port}/api/importer/upload failed");
    assert!(message.starts_with(&expected), "{message}");
    assert!(!message.contains("password"), "{message}");
}

// The data dictionary

#[test]
fn the_data_dictionaries_are_fetched_without_the_login() {
    let request = dkan().dictionaries_request().unwrap();

    assert_eq!(request.method(), Method::GET);
    assert_eq!(
        request.url().as_str(),
        "https://dkan.example/api/1/metastore/schemas/data-dictionary/items"
    );
    assert_eq!(header(&request, header::ACCEPT), Some("application/json"));
    assert_eq!(header(&request, header::AUTHORIZATION), None);
}

#[test]
fn a_failed_fetch_of_the_data_dictionaries_gives_its_status_and_body() {
    let message = dictionaries("https://dkan.example/list", &response(401, "Access denied"))
        .expect_err("a 401 must be an error")
        .to_string();

    let expected = "Failed to fetch the data dictionaries from https://dkan.example/list: HTTP 401 Unauthorized: Access denied";
    assert_eq!(message, expected);
}

#[test]
fn the_data_dictionaries_are_the_body_of_a_successful_answer() {
    let answer = response(200, "[]");

    assert_eq!(
        dictionaries("https://dkan.example/list", &answer).unwrap(),
        "[]"
    );
}

#[test]
fn a_data_dictionary_is_found_when_its_own_url_answers() {
    assert!(dictionary_found("dd-1", DICTIONARY_URL, &response(200, "{}")).is_ok());

    let message = dictionary_found("dd-1", DICTIONARY_URL, &response(404, ""))
        .expect_err("a 404 must be an error")
        .to_string();
    assert!(message.contains("data dictionary dd-1"), "{message}");
    assert!(message.contains(DICTIONARY_URL), "{message}");
}

// The upload

#[test]
fn the_upload_posts_the_csv_as_a_named_file_with_the_login() {
    let request = dkan()
        .upload_request("data.csv", b"a,b\n1,2\n".to_vec())
        .unwrap();

    assert_eq!(request.method(), Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://dkan.example/api/importer/upload"
    );
    assert_eq!(header(&request, header::AUTHORIZATION), Some(BASIC_AUTH));
    let content_type = header(&request, header::CONTENT_TYPE).unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap_or_else(|| panic!("not a multipart form: {content_type}"));
    let body = String::from_utf8(body(&request).to_vec()).unwrap();
    let expected = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"csv\"; filename=\"data.csv\"\r\nContent-Type: text/csv\r\n\r\na,b\n1,2\n\r\n--{boundary}--\r\n"
    );
    assert_eq!(body, expected);
}

#[test]
fn the_uploaded_files_url_is_read_from_the_answer() {
    let answer = response(200, r#"{"data":{"file_url":"https://dkan.example/f.csv"}}"#);

    assert_eq!(
        uploaded_file_url(&answer).unwrap(),
        "https://dkan.example/f.csv"
    );
}

#[test]
fn an_upload_answer_without_a_file_url_is_an_error_not_a_panic() {
    let message = uploaded_file_url(&response(200, r#"{"data":{}}"#))
        .expect_err("an answer without file_url must be an error")
        .to_string();

    assert_eq!(
        message,
        r#"The upload response has no data.file_url: {"data":{}}"#
    );
}

#[test]
fn a_rejected_upload_gives_the_status_without_the_page() {
    let message = uploaded_file_url(&response(403, ACCESS_DENIED_PAGE))
        .expect_err("a 403 must be an error")
        .to_string();

    assert_eq!(message, "Custom importer upload failed: HTTP 403 Forbidden");
}

#[test]
fn a_failed_upload_gives_the_importers_error_and_details() {
    let message = uploaded_file_url(&response(
        500,
        r#"{"error":"An error occurred while uploading the file.","status":"error","details":"disk full"}"#,
    ))
    .expect_err("a 500 must be an error")
    .to_string();

    let expected = "Custom importer upload failed: HTTP 500 Internal Server Error: An error occurred while uploading the file. (disk full)";
    assert_eq!(message, expected);
}

// Adding the distribution

#[test]
fn the_dataset_is_read_and_saved_at_its_url_with_the_login() {
    let dataset = json!({"title": "T"});

    let get = dkan().dataset_request(Method::GET, "ds/1").build().unwrap();
    let patch = dkan()
        .dataset_request(Method::PATCH, "ds/1")
        .json(&dataset)
        .build()
        .unwrap();

    let expected_url = "https://dkan.example/api/1/metastore/schemas/dataset/items/ds%2F1";
    assert_eq!(get.method(), Method::GET);
    assert_eq!(get.url().as_str(), expected_url);
    assert_eq!(header(&get, header::AUTHORIZATION), Some(BASIC_AUTH));
    assert_eq!(patch.method(), Method::PATCH);
    assert_eq!(patch.url().as_str(), expected_url);
    assert_eq!(header(&patch, header::AUTHORIZATION), Some(BASIC_AUTH));
    assert_eq!(
        header(&patch, header::CONTENT_TYPE),
        Some("application/json")
    );
    let sent: Value = serde_json::from_slice(body(&patch)).unwrap();
    assert_eq!(sent, dataset);
}

/// The distribution DKAN gets for `new_distribution()`
fn added_distribution() -> Value {
    json!({
        "title": "new.csv",
        "description": "Data file: new.csv",
        "format": "csv",
        "mediaType": "text/csv",
        "downloadURL": "https://dkan.example/files/new.csv",
        "describedBy": DICTIONARY_URL,
        "describedByType": "application/vnd.tableschema+json",
    })
}

#[test]
fn a_dataset_without_distributions_gets_the_new_one_keeping_its_other_fields() {
    let answer = response(200, r#"{"title":"Arctic","keyword":["water"]}"#);

    let (dataset, added) = dataset_with_distribution("ds-1", &answer, new_distribution()).unwrap();

    let expected = json!({
        "title": "Arctic",
        "keyword": ["water"],
        "distribution": [added_distribution()],
    });
    assert_eq!(dataset, expected);
    let expected = DistributionAdded {
        dataset_title: "Arctic".to_string(),
        previous_csv_filename: None,
    };
    assert_eq!(added, expected);
}

#[test]
fn the_distribution_of_the_same_data_dictionary_is_replaced_and_the_others_kept() {
    let other = json!({"title": "other.csv", "describedBy": "https://dkan.example/other"});
    let answer = response(
        200,
        &json!({
            "title": "Arctic",
            "distribution": [
                other,
                {"title": "old.csv", "describedBy": DICTIONARY_URL},
            ],
        })
        .to_string(),
    );

    let (dataset, added) = dataset_with_distribution("ds-1", &answer, new_distribution()).unwrap();

    assert_eq!(
        dataset["distribution"],
        json!([other, added_distribution()])
    );
    assert_eq!(added.previous_csv_filename.as_deref(), Some("old.csv"));
}

#[test]
fn a_replaced_distribution_without_a_title_is_named_by_its_download_url() {
    let answer = response(
        200,
        &json!({
            "title": "Arctic",
            "distribution": [{
                "downloadURL": "https://dkan.example/files/old.csv",
                "describedBy": DICTIONARY_URL,
            }],
        })
        .to_string(),
    );

    let (_, added) = dataset_with_distribution("ds-1", &answer, new_distribution()).unwrap();

    assert_eq!(added.previous_csv_filename.as_deref(), Some("old.csv"));
}

#[test]
fn a_dataset_that_cannot_be_read_is_an_error() {
    let cases = [
        (
            response(404, "no such dataset"),
            "Failed to get dataset ds-1: no such dataset",
        ),
        (response(200, "<html>"), "The dataset ds-1 is not JSON"),
        (
            response(200, r#"{"distribution":[]}"#),
            "Dataset title not found",
        ),
    ];
    for (answer, expected) in cases {
        let message = dataset_with_distribution("ds-1", &answer, new_distribution())
            .expect_err("the dataset must be refused")
            .to_string();

        assert_eq!(message, expected);
    }
}

#[test]
fn a_dataset_not_saved_is_an_error_naming_it() {
    let added = DistributionAdded {
        dataset_title: "Arctic".to_string(),
        previous_csv_filename: None,
    };
    assert!(distribution_saved("ds-1", &added, &response(200, "{}")).is_ok());

    let message = distribution_saved("ds-1", &added, &response(422, "bad distribution"))
        .expect_err("a 422 must be an error")
        .to_string();

    let expected = "Failed to add CSV distribution to dataset \"Arctic\" with id \"ds-1\" with error: bad distribution";
    assert_eq!(message, expected);
}

// Deleting the previous file

#[test]
fn a_file_is_deleted_by_a_post_to_its_url_with_the_login() {
    let request = dkan().delete_request("old file.csv").unwrap();

    assert_eq!(request.method(), Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://dkan.example/api/importer/delete/old%20file.csv"
    );
    assert_eq!(header(&request, header::AUTHORIZATION), Some(BASIC_AUTH));
}

#[test]
fn a_file_not_deleted_is_an_error_naming_it() {
    assert!(file_deleted("old.csv", &response(200, "")).is_ok());

    let message = file_deleted("old.csv", &response(404, "not found"))
        .expect_err("a 404 must be an error")
        .to_string();

    assert_eq!(message, "Failed to delete file old.csv: not found");
}
