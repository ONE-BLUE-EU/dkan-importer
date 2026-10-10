# Backlog

Findings from reviewing the code against [ENGINEERING_STANDARDS.md](ENGINEERING_STANDARDS.md) and
[SECURITY.md](SECURITY.md) on 2026-10-03. Ordered by priority.

## Open

- [ ] **Coordinate detection matches substrings of unrelated field names** (§3).
  `crates/excel-core/src/excel_validator.rs` treats any field whose name contains `lat`, `lon` or `lng`
  as a coordinate, so "Plate count", "Isolate", "Colony" or "Salinity class" are sent through the
  coordinate parser. Example: a string field "Isolate code" with the value `12` is parsed as
  latitude 12.0 and then fails validation as a number in a string column. Match whole words, or
  take the coordinate columns from the data dictionary.

## Done (2026-10-03)

- [x] **Empty numeric cells are uploaded as zero.** Kept on purpose, because DKAN infers column types
  from the CSV. Now documented on `NUMERIC_PLACEHOLDER` / `INTEGER_PLACEHOLDER` and in
  `crates/dkan-importer/README.md`, and pinned by `tests/excel_file_pipeline_test.rs`.
- [x] **Excel error cells (`#N/A`, ...) were uploaded as zero.** They now stop the import with a list of
  the affected cells.
- [x] **Ambiguous dates.** Numeric dates are read day-first only. Month-first dates, and any other
  text in a `date`/`date-time` column, are now validation errors (previously uploaded as-is).
- [x] **Failures writing `errors.log` were ignored.** `write_error_to_log` now returns an error, and
  callers put the details in the error message when the log cannot be written.
- [x] **Data dictionary download didn't check the status and sent a placeholder header.** It now
  checks the status and no longer sends the `Bearer <token>` header. Tested against a local stub
  server.
- [x] **Panics where an error should be returned.** Fixed in `main.rs` (non-HTTPS URL, password prompt,
  CSV export), `utils.rs` (upload response without `file_url`) and `data_dictionary.rs`
  (dictionary without a title).
- [x] **Pivoter wrote dates as serial numbers.** It now applies a date format. Added the pivoter's
  first tests.
- [x] **Placeholder tests** (`assert!(true)` and one with no assertions) deleted.
- [x] **`mod tests` without `#[cfg(test)]`** fixed.
- [x] **`multipleOf` rounding that silently changed values** removed. Values are now validated
  against `multipleOf` as they are, with the existing floating-point tolerance.
- [x] **Schema errors swallowed in `extract_field_schemas`** — errors are now propagated.
- [x] **Clippy warnings** fixed. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [x] **`pivoter` was committed as a broken submodule pointer**, so its source was never in the
  repository. It is now tracked as normal files.

## Deliberately not planned

Not worth the cost for an internal import tool: the clippy restriction lints and fuzzing (the panic
fixes above cover the real risk), memory tuning (§10), and splitting `excel_validator.rs`. Adding
`#![forbid(unsafe_code)]` is a one-line change per crate if wanted; there is no `unsafe` code.
