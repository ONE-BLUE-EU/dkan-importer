//! Tests of the command line, run as the real binary. Every run points at a closed local port, so
//! nothing reaches a real host.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The password every run is given; it must never reach the run log.
const PASSWORD: &str = "pw-never-logged-7f3a";

/// An https URL on a local port nothing listens on.
fn closed_https_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("https://{}", listener.local_addr().unwrap())
}

/// A new empty directory for one run, so that what it writes stays out of the repository and
/// away from other tests.
fn new_run_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("dkan-importer-cli-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run dkan-importer with the required arguments followed by `extra`, in a directory of its own.
fn run(extra: &[&str]) -> Output {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let dir = new_run_dir(&format!("run-{}", RUNS.fetch_add(1, Ordering::Relaxed)));
    let output = run_in(&dir, extra);
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// Run dkan-importer with the required arguments followed by `extra`, in `dir`.
fn run_in(dir: &Path, extra: &[&str]) -> Output {
    let base_url = closed_https_url();
    Command::new(env!("CARGO_BIN_EXE_dkan-importer"))
        .current_dir(dir)
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
            PASSWORD,
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
fn values_markers_without_unpivot_is_refused() {
    let output = run(&["--values-markers", "<MDL"]);

    assert_usage_error(&output, "--unpivot");
}

/// Several markers follow one flag, and the run gets past the arguments to the login check.
#[test]
fn unpivot_with_several_values_markers_is_accepted() {
    let output = run(&[
        "--unpivot",
        "--headers-column",
        "Sample Tag",
        "--values-markers",
        "<MDL",
        "<LOD",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_exit_code = Some(1);
    assert_eq!(output.status.code(), expected_exit_code, "{stderr}");
    assert!(stderr.contains("/api/importer/upload"), "{stderr}");
}

/// Refused before anything runs, even when given on purpose.
#[test]
fn nd_as_a_values_marker_is_refused_as_it_gives_no_row() {
    let output = run(&[
        "--unpivot",
        "--headers-column",
        "Sample Tag",
        "--values-markers",
        "<LOD",
        "ND",
    ]);

    assert_usage_error(
        &output,
        "\"ND\" cannot be a values marker: ND cells are treated as NS (no sample) and give no row.",
    );
}

#[test]
fn lq_as_a_values_marker_is_refused_pointing_to_loq() {
    let output = run(&[
        "--unpivot",
        "--headers-column",
        "Sample Tag",
        "--values-markers",
        "<LQ",
    ]);

    assert_usage_error(
        &output,
        "\"<LQ\" cannot be a values marker: <LQ maps to <LOQ. Use <LOQ, and change <LQ to <LOQ in the source file, for consistency.",
    );
}

#[test]
fn nq_as_a_values_marker_is_refused_pointing_to_loq() {
    let output = run(&[
        "--unpivot",
        "--headers-column",
        "Sample Tag",
        "--values-markers",
        "NQ",
    ]);

    assert_usage_error(
        &output,
        "\"NQ\" cannot be a values marker: NQ maps to <LOQ. Use <LOQ, and change NQ to <LOQ in the source file, for consistency.",
    );
}

#[test]
fn marker_limits_without_unpivot_is_refused() {
    let output = run(&["--marker-limits", "<LOQ=Limit of quantification"]);

    assert_usage_error(&output, "--unpivot");
}

#[test]
fn a_marker_limit_without_a_field_is_refused() {
    let output = run(&[
        "--unpivot",
        "--headers-column",
        "Sample Tag",
        "--marker-limits",
        "<LOQ",
    ]);

    assert_usage_error(&output, "expected MARKER=FIELD");
}

/// Several limits follow one flag, and the run gets past the arguments to the login check.
#[test]
fn unpivot_with_several_marker_limits_is_accepted() {
    let output = run(&[
        "--unpivot",
        "--headers-column",
        "Sample Tag",
        "--marker-limits",
        "<LOQ=Limit of quantification",
        "<LOD=Limit of detection",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_exit_code = Some(1);
    assert_eq!(output.status.code(), expected_exit_code, "{stderr}");
    assert!(stderr.contains("/api/importer/upload"), "{stderr}");
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
        unpivot_section.contains("--values-markers <MARKER>..."),
        "{help}"
    );
    assert!(unpivot_section.contains("[default: <LOD <LOQ X]"), "{help}");
    assert!(
        unpivot_section.contains("--marker-limits <MARKER=FIELD>..."),
        "{help}"
    );
    assert!(
        unpivot_section
            .contains("[default: \"<LOD=Limit of detection\" \"<LOQ=Limit of quantification\"]"),
        "{help}"
    );
    assert!(
        unpivot_section.contains(
            "--unpivot --headers-column \"Sample Tag\" --values-column \"Concentration\""
        ),
        "{help}"
    );
}

#[test]
fn extra_columns_to_with_unpivot_is_refused() {
    let output = run(&[
        "--extra-columns-to",
        "Remark 3",
        "--unpivot",
        "--headers-column",
        "Sample Tag",
    ]);

    assert_usage_error(&output, "--unpivot");
}

/// The field is optional: `--extra-columns-to` alone gets past the arguments and stops at the
/// first network step, the login check.
#[test]
fn extra_columns_to_without_a_field_is_accepted() {
    let output = run(&["--extra-columns-to"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_exit_code = Some(1);
    assert_eq!(output.status.code(), expected_exit_code, "{stderr}");
    assert!(stderr.contains("/api/importer/upload"), "{stderr}");
}

#[test]
fn help_shows_extra_columns_to_with_its_default_field() {
    let output = Command::new(env!("CARGO_BIN_EXE_dkan-importer"))
        .arg("--help")
        .output()
        .unwrap();

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{help}");
    assert!(help.contains("--extra-columns-to [<FIELD>]"), "{help}");
    assert!(
        help.contains("[default when given alone: Remark 1]"),
        "{help}"
    );
}

/// A run that fails still leaves its log, with what it was given and why it stopped, but never
/// the password.
#[test]
fn a_failed_run_leaves_a_run_log_with_the_error_and_without_the_password() {
    let dir = new_run_dir("run-log-failed");

    let output = run_in(&dir, &["--sheet-name", "Analytical results (I)"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_exit_code = Some(1);
    assert_eq!(output.status.code(), expected_exit_code, "{stderr}");
    let log_path = dir.join("some-dataset_analytical_results_(i).run.log");
    let log = std::fs::read_to_string(&log_path)
        .unwrap_or_else(|e| panic!("no run log at {}: {e}", log_path.display()));
    assert!(log.contains("Excel file: unused.xlsx\n"), "{log}");
    assert!(log.contains("Sheet: Analytical results (I)\n"), "{log}");
    let error_line = stderr
        .lines()
        .find(|line| line.starts_with("❌") && line.contains("/api/importer/upload"))
        .unwrap_or_else(|| panic!("no error line in:\n{stderr}"));
    assert!(log.contains(error_line), "{error_line}\n{log}");
    assert!(!log.contains(PASSWORD), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The run log always describes the latest run of a dataset and sheet.
#[test]
fn each_run_replaces_the_run_log_of_the_last_one() {
    let dir = new_run_dir("run-log-replaced");
    let log_path = dir.join("some-dataset_sheet1.run.log");
    std::fs::write(&log_path, "from an earlier run\n").unwrap();

    run_in(&dir, &[]);

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(!log.contains("from an earlier run"), "{log}");
    assert!(log.contains("Sheet: Sheet1\n"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}
