# Security Guidelines

Client-side security rules for this workspace. Condensed from `~/projects/engineering-standards-core/`
(section numbers match it). Baseline: OWASP ASVS Level 2.

## 2. Authentication
- Never log or write credentials. Prefer the password prompt over `--password` (shell history).
- Basic auth only to `https://` URLs; reject other schemes before any request.

## 10. Upstream APIs
- DKAN responses are untrusted: parse into types (§3 of the standards) and set client timeouts.
- Never disable TLS certificate verification.

## Cross-Cutting
- Input is data at every boundary (file paths, shell commands).
- No hardcoded or committed secrets, and none in logs or error messages.
- Dependency versions live in one place, with a committed `Cargo.lock`.
- Advisory scanning runs in the local checks: use a cached database, report its age, warn loudly when offline, and record a reason and review date for every `ignore`. CI re-runs it on a schedule.
- Anything not covered here: follow ASVS L2. Record any deviation as an ADR.
