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

## Open questions to data owners (Analytical results)

Raised by the unpivoter as "Questions to data owners" in the run logs of 2026-11-07/08. Each needs an answer, then a change to the unpivoter and a re-import of the
affected datasets. Points 4 and 5 are data left out of DKAN. Points 1–3 are already uploaded using
the assumption described.

- [ ] **"detected, not quantified" is not in the template's dropdown.** Results between the Limit of
  detection and the Limit of quantification, and `<LOQ`/`<MLOQ` results, are imported with Detection
  characterisation "detected, not quantified". The template has "detected semiquantitatively". Which
  one should be used? 3,640 cells: Arctic Ocean 2023 (I) 1,389; Spanish Coast 2025 1,344 (column water
  902, PFAS 366, Sediment 56, MNPLs water 11, Air 8, column wa-1 1); Ionian Sea 2025 886; Arctic Ocean
  2024 (II) 14; Iberian Peninsula 2026 Marine Toxins 7.
- [ ] **Numbers in rows with no Limit of quantification** are imported as "detected quantitatively",
  unless they are below the row's Limit of detection. Is that correct? 3,274 cells, Spanish Coast 2025:
  PFAS 2,190, Sediment 905, Air 179.
- [ ] **`X` in rows with no Concentration units** is imported as Peak Area (detected qualitatively),
  and the units are left blank. Is that correct? 116 cells: Spanish Coast 2025 (Sediment 88, MNPLs
  water 5, Air 2); Tyrrhenian Sea 2025 MNPLs 21.
- [ ] **µg/L rows are not imported.** How should they be handled (for example, converted to ng/L)? 42
  cells, Tyrrhenian Sea 2025, MNPLs sheet rows 2–5.
- [ ] **ng/L rows with no Limit of detection and no Limit of quantification are not imported**,
  because without either limit their results can't be characterised. Should they be imported, and if
  so, how? 69 cells, Arctic Ocean 2023, Analytical results (I).

## Done (2026-10-03)

- [x] **Empty numeric cells are uploaded as zero.** Kept on purpose, because DKAN infers column types
  from the CSV. Now documented on `NUMERIC_PLACEHOLDER` / `INTEGER_PLACEHOLDER` and in
  `crates/dkan-importer/README.md`, and pinned by `tests/excel_file_pipeline_test.rs`.
- [x] **Excel error cells (`#N/A`, ...) were uploaded as zero.** They now stop the import with a list of
  the affected cells. *Reversed on 2026-11-05 at the data owner's request:* error cells are read as blank
  cells again (so a numeric one is uploaded as the placeholder), and the run lists every one of them.
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
