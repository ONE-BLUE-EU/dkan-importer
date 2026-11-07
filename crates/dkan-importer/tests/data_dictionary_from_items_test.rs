//! Reading a data dictionary from DKAN's list of data dictionaries, without a network.

use dkan_importer::dkan::HttpsUrl;
use dkan_importer::model::DataDictionary;
use excel_core::serde_json::json;

fn base_url() -> HttpsUrl {
    "https://dkan.example".parse().unwrap()
}

fn items(data: excel_core::serde_json::Value) -> String {
    json!([
        {"identifier": "other", "data": {"title": "Other", "fields": []}},
        {"identifier": "dd-1", "data": data},
    ])
    .to_string()
}

#[test]
fn the_data_dictionary_with_the_identifier_is_read_with_its_fields_normalized() {
    let body = items(json!({
        "title": "Samples",
        "fields": [{"name": "  depth ", "title": "Depth\t(m)", "type": "number"}],
    }));

    let dictionary = DataDictionary::from_items(&base_url(), "dd-1", &body).unwrap();

    assert_eq!(dictionary.id, "dd-1");
    assert_eq!(dictionary.name, "Samples");
    assert_eq!(
        dictionary.url,
        "https://dkan.example/api/1/metastore/schemas/data-dictionary/items/dd-1"
    );
    let expected = json!({
        "title": "Samples",
        "fields": [{"name": "depth", "title": "Depth (m)", "type": "number"}],
    });
    assert_eq!(dictionary.fields, expected);
}

#[test]
fn a_data_dictionary_that_cannot_be_read_is_an_error() {
    let cases = [
        (
            items(json!({"title": "Samples", "fields": []})).replace("dd-1", "dd-2"),
            "Data dictionary with identifier 'dd-1' not found",
        ),
        (
            json!([{"identifier": "dd-1"}]).to_string(),
            "Data dictionary data not found",
        ),
        (
            items(json!({"fields": []})),
            "Data dictionary 'dd-1' has no title",
        ),
        (
            "<html>".to_string(),
            "The list of data dictionaries is not a JSON array",
        ),
    ];
    for (body, expected) in cases {
        let message = DataDictionary::from_items(&base_url(), "dd-1", &body)
            .err()
            .expect("the data dictionary must be refused")
            .to_string();

        assert!(message.starts_with(expected), "{message}");
    }
}

#[test]
fn a_data_dictionary_with_fields_duplicated_once_normalized_is_refused() {
    let body = items(json!({
        "title": "Samples",
        "fields": [
            {"name": "depth", "title": "Depth", "type": "number"},
            {"name": " depth", "title": "Depth ", "type": "number"},
        ],
    }));

    let message = DataDictionary::from_items(&base_url(), "dd-1", &body)
        .err()
        .expect("the duplicates must be refused")
        .to_string();

    assert!(
        message.starts_with("Data dictionary contains duplicate fields:"),
        "{message}"
    );
    assert!(
        message.contains("Field name 'depth' appears at positions: 1, 2"),
        "{message}"
    );
}
