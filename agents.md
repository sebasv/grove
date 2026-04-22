# Agent Directives

## Working on PRs

Any work on a PR is not complete until CI passes. Before considering a PR done:

- Run `cargo fmt --check` and fix any formatting issues with `cargo fmt`.
- Run `cargo clippy -- -D warnings` and fix all warnings without using `#[allow(...)]`.
- Run `cargo test` and ensure all tests pass.
- Push the fixes and confirm CI is green before marking work as complete or merging.

## CI Fixes

When fixing CI failures (clippy, fmt, tests):

- **Do not suppress warnings.** Never add `#[allow(...)]` attributes to silence clippy lints. Fix the underlying code instead.
- **Do not make functional changes to fix CI.** Formatting and lint fixes must be purely structural — rename, reorder, collapse, derive — without altering runtime behavior.
