# Backlog

Findings from reviewing the code against [ENGINEERING_STANDARDS.md](ENGINEERING_STANDARDS.md) and
[SECURITY.md](SECURITY.md) on 2026-10-03. Ordered by priority. Line numbers are as of that date.

## High — wrong data reaches DKAN

- [ ] **Empty numeric cells are uploaded as zero** (§5, §9).
  `export_to_csv` (`importer-lib/src/excel_validator.rs:375-397`) writes `000000000000.000000` for
  an empty Number cell and `0` for an empty Integer cell, so a missing measurement is published as a
  real 0. Excel error cells (`#N/A`, `#DIV/0!`) become null first (`:695`) and end up as 0 too. No
  test covers this. If DKAN needs a type hint, give it another way.
- [ ] **Ambiguous dates are silently read as day/month** (§3, §9).
  `importer-lib/src/utils/datetime.rs:24-30` tries `%d/%m/%Y` before `%m/%d/%Y`, so `03/04/2024`
  is always 3 April and the US format is used only when the day is over 12. Reject dates that could
  be read both ways, or let the user specify the format.

## Medium — failures that are hidden or unclear

- [ ] **Failures writing `errors.log` are ignored** (§5).
  `importer-lib/src/utils/filesystem.rs:14-20` drops open and write errors, while `main` tells the
  user to check the file.
- [ ] **Data dictionary download doesn't check the status and sends a placeholder header** (§5, SECURITY §10).
  `dkan-importer/src/model/data_dictionary.rs:22-27`: a 401 or 500 shows up as a JSON parse error,
  and the request sends the literal header `Authorization: Bearer <token>`.
- [ ] **Panics where an error should be returned** (§11). The release build aborts on panic:
  - `dkan-importer/src/main.rs:64` — URL isn't HTTPS
  - `dkan-importer/src/main.rs:108` — CSV export failed
  - `dkan-importer/src/utils.rs:199` — `expect("File URL not found")` on DKAN's upload response

## Low

- [ ] **Pivoter writes dates as raw serial numbers** — `pivoter/src/lib.rs:136` has no date format,
  so the output shows values like `45678`. The pivoter also has no tests (§7).
- [ ] **A test that can't fail** — `assert!(true, ...)` at `importer-lib/src/excel_validator.rs:3086` (§7).
- [ ] **`mod tests` without `#[cfg(test)]`** in `dkan-importer/src/utils.rs:202`.
- [ ] **Unused `multipleOf` rounding** (`importer-lib/src/excel_validator.rs:1258`). The data dictionary
  conversion never produces `multipleOf`, so this is dead code that would change values (§1).
- [ ] **Schema errors are swallowed** — `extract_field_schemas` (`importer-lib/src/excel_validator.rs:438`)
  ignores fields whose schema it can't parse.
- [ ] **Fix the ~10 clippy warnings** (`cargo clippy --workspace --all-targets`) (§11).

## Deliberately not planned

Not worth the cost for an internal import tool: the clippy restriction lints and fuzzing (the panic
fixes above cover the real risk), memory tuning (§10), and splitting `excel_validator.rs`. Adding
`#![forbid(unsafe_code)]` is a one-line change per crate if wanted; there is no `unsafe` code.
