# DKAN Importer Workspace

A Cargo workspace of tools for getting Excel data into DKAN: one shared library and two command-line binaries.

| Crate | Kind | Binary | Purpose |
| ----- | ---- | ------ | ------- |
| [`importer-lib`](importer-lib/) | library | — | Shared Excel validation, value conversion and CSV/Excel writers |
| [`dkan-importer`](dkan-importer/) | binary | `dkan-importer` | Validates an Excel file against a DKAN data dictionary and uploads it to a dataset |
| [`pivoter`](pivoter/) (package `unpivoter`) | binary | `upr` | Unpivots wide Excel sheets into long format |

## Binaries

### `dkan-importer`

Fetches a data dictionary from a DKAN instance, converts it to JSON Schema, and checks each row of an Excel sheet against it. It writes the valid rows to a timestamped CSV file and uploads that file as a distribution of an existing DKAN dataset. Rows that fail validation are reported in `errors.log`.

```bash
cargo run -r -p dkan-importer -- \
  --base-url https://dkan.example.com \
  --excel-file data.xlsx \
  --data-dictionary-id <DICTIONARY_UUID> \
  --dataset-id <DATASET_UUID> \
  --username admin
```

See [dkan-importer/README.md](dkan-importer/README.md) for all options, the data dictionary type mapping, and the output formats.

### `upr` (unpivoter)

Converts a wide sheet (one column per measurement) into a long sheet with one row per measurement. Columns before `--separator-column` are treated as ID columns and copied to every output row. Every column from that index onward becomes its own row: the column header goes into a `Sample Tag` column and the cell goes into a `_value_` column. Cell types are preserved. Columns that contain only 0/1 values are found automatically (or set with `--bool-columns`) and written as `FALSE`/`TRUE`.

```bash
cargo run -r -p unpivoter -- \
  --excel-file IrishSea_analytical_results_water.xlsx \
  --sheet "Sheet1" \
  --separator-column 11
```

The output is written to `<file>_<sheet>.xlsx` in the current directory, in a sheet named `Analytical results`. Add `--debug` to print the detected cell types and boolean columns. Run `cargo run -p unpivoter -- --help` to see all options.

## Shared library: `importer-lib`

Code shared by the binaries:

- `ExcelValidator` validates Excel rows against a schema and coerces values to the expected types.
- `utils` provides date/time parsing, coordinate parsing, string normalization and value conversion. See [COORDINATE_PARSING_INTEGRATION.md](COORDINATE_PARSING_INTEGRATION.md) for how latitude/longitude values are handled.
- `writers` contains the CSV and Excel output writers.
- It re-exports `anyhow`, `log`, `reqwest`, `serde` and `serde_json`, so dependent crates use the same versions.

## Engineering standards

All work in this workspace follows [ENGINEERING_STANDARDS.md](ENGINEERING_STANDARDS.md) and [SECURITY.md](SECURITY.md). Both are adapted from the shared `engineering-standards-core` and keep only the rules that apply to these tools.

## Building

All crates share one `target/` directory and one `Cargo.lock`. Dependency versions are declared once in the root [`Cargo.toml`](Cargo.toml) under `[workspace.dependencies]`, and each crate refers to them with `<crate>.workspace = true`. The release profile (fat LTO, one codegen unit, stripped binaries, `panic = "abort"`) is also set in the root manifest.

```bash
cargo build --release              # build everything
cargo build --release -p unpivoter # build one crate
cargo test --workspace             # run all tests
```

Release binaries are written to `target/release/` (`dkan-importer`, `upr`).
