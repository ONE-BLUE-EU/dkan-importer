//! Tests of the command line, run as the real binary. Every run points at a closed local port, so
//! nothing reaches a real host.

use std::net::TcpListener;
use std::process::{Command, Output};

/// An https URL on a local port nothing listens on.
fn closed_https_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("https://{}", listener.local_addr().unwrap())
}

/// Run dkan-importer with the required arguments followed by `extra`.
fn run(extra: &[&str]) -> Output {
    let base_url = closed_https_url();
    Command::new(env!("CARGO_BIN_EXE_dkan-importer"))
        .args([
            "--base-url",
            &base_url,
            "--excel-file",
            "unused.xlsx",
            "--data-dictionary-id",
            "some-id",
            "--username",
            "user",
            "--password",
            "password",
            "--dataset-id",
            "some-dataset",
        ])
        .args(extra)
        .output()
        .unwrap()
}

/// Assert that clap refused the arguments as a usage error (exit code 2, before anything runs) and
/// that its message names `expected`.
fn assert_usage_error(output: &Output, expected: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_exit_code = Some(2);
    assert_eq!(output.status.code(), expected_exit_code, "{stderr}");
    assert!(stderr.contains(expected), "{stderr}");
}

#[test]
fn headers_column_without_unpivot_is_refused() {
    let output = run(&["--headers-column", "Sample Tag"]);

    assert_usage_error(&output, "--unpivot");
}

#[test]
fn values_column_without_unpivot_is_refused() {
    let output = run(&["--values-column", "Concentration"]);

    assert_usage_error(&output, "--unpivot");
}

#[test]
fn unpivot_without_headers_column_is_refused() {
    let output = run(&["--unpivot"]);

    assert_usage_error(&output, "--headers-column <FIELD>");
}

/// `--values-column` has a default, so `--unpivot --headers-column` is a complete unpivot request:
/// the run gets past the arguments and stops at the first network step, the login check.
#[test]
fn unpivot_with_headers_column_is_accepted() {
    let output = run(&["--unpivot", "--headers-column", "Sample Tag"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_exit_code = Some(1);
    assert_eq!(output.status.code(), expected_exit_code, "{stderr}");
    assert!(stderr.contains("/api/importer/upload"), "{stderr}");
}

#[test]
fn help_groups_the_unpivot_flags_with_their_default_and_an_example() {
    let output = Command::new(env!("CARGO_BIN_EXE_dkan-importer"))
        .arg("--help")
        .output()
        .unwrap();

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{help}");
    let unpivot_section = help
        .split_once("Unpivot:\n")
        .map(|(_, section)| section)
        .unwrap_or_else(|| panic!("no Unpivot section in:\n{help}"));
    assert!(unpivot_section.contains("--unpivot"), "{help}");
    assert!(
        unpivot_section.contains("--headers-column <FIELD>"),
        "{help}"
    );
    assert!(
        unpivot_section.contains("--values-column <FIELD>"),
        "{help}"
    );
    assert!(
        unpivot_section.contains("[default: Concentration]"),
        "{help}"
    );
    assert!(
        unpivot_section.contains(
            "--unpivot --headers-column \"Sample Tag\" --values-column \"Concentration\""
        ),
        "{help}"
    );
}
