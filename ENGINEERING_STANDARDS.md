# Engineering Standards

Rules all work in this workspace must follow. Condensed from `~/projects/engineering-standards-core/`
(section numbers match it; §0, §6, §8 and §12.1–12.3 do not apply here). Bring improvements back there.

## 1. Simplicity First
Simplest solution that fully solves the problem wins over every other rule. Solve the problem in front of you, check what already exists, fix causes not symptoms, prefer deleting. Growing complexity means re-read the requirement.

## 2. Functional Core, Imperative Shell
Logic is pure functions (no I/O, clock, or ambient config) in `importer-lib` and `pivoter/src/lib.rs`. Each binary's `main` is a thin shell: parse input, call the core, perform side effects.

## 3. Parse, Don't Validate
At every boundary (CLI args, Excel cells, DKAN responses) convert input into types that can only hold valid values. Don't check and then pass the raw `String` on.

## 4. One Source of Truth
Shared code lives in `importer-lib`; dependency versions in the root `Cargo.toml`. Tests import production constants and helpers — no parallel copies.

## 5. Explicit Errors
Startup failures (DKAN unreachable, bad login, bad data dictionary, missing sheet) stop the run with a named error and non-zero exit before anything is written or uploaded. Never default to empty or zero.

## 7. Testing
- Prefer making an invalid state impossible to construct over testing for it; test where the value is constructed.
- Bug → failing test first. Watch it fail for the right reason.
- Test observable behaviour, not implementation. Integration tests use only the public API or the CLI.
- No placeholder or silently skipped tests.
- Ask of every assertion: would it pass under another correct implementation? Would it pass with the feature switched off? Does it pass on the default state before the action?
- An "is absent" assertion needs a matching "is present" case.
- Name the expected value, then compare — equality is satisfied by both sides being wrong.
- List every outcome from the code and name the test for each; refusals are the ones usually missed.
- After moving code into a shared crate, break it and confirm each consumer's suite fails.
- Report the exact command you ran, not "the tests pass".
- For scripted edits, mutations and searches: assert the pattern matched and the path exists, confirm the change reached the tool, and read *why* a run failed, not just its exit code.
- Don't let a pipe hide an exit status. Make sure a check can actually fail.
- Distrust comments that claim a property ("always", "never") or describe another tool — verify them.

## 9. Type-Driven Design
Design types first. Make illegal states unrepresentable (sum types over flags and optional fields) and illegal transitions unconstructable (private constructors). Count states before using `bool`. Newtypes for interchangeable arguments. Return things that must travel together, together. `None` instead of a plausible default. Take requirements (URLs, paths) as arguments. If a rule forces contorted code, question the rule. Use typestate only for real ordering problems.

## 10. Rust Memory Efficiency
Borrow rather than clone; use `Cow`/`Rc`/`Arc` where they fit. Consider `smallvec`/`compact_str` for small data. Write idiomatic code first and profile before optimising. Order struct fields largest to smallest; use `NonZero`.

## 11. Rust Directives
- `#![forbid(unsafe_code)]`.
- Clippy warnings are errors.
- Nothing that processes input may panic (release uses `panic = "abort"`): no `unwrap`, `expect`, indexing or unchecked arithmetic. Enforce with types, then the clippy lints `unwrap_used`, `expect_used`, `indexing_slicing`, `string_slice` and `arithmetic_side_effects`, then fuzzing of input parsers. `#[allow]` only with a reason. Fix every site in a crate in one change.
- `cargo fmt` before commit.
- No unused dependencies (`cargo-shear`); any ignored dependency gets a written reason.
- Release profile lives at the workspace root.

## 12. Test Infrastructure
- Tests never call real or unresolvable hosts; point them at a closed local port or a mock server.
- Tests don't depend on each other's state or on shared files such as `errors.log`.
- Know what your test runner hides (passing-test output). Make retries and fallbacks print a line.

See [SECURITY.md](SECURITY.md).
