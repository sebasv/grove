# Agent Directives

## CI Fixes

When fixing CI failures (clippy, fmt, tests):

- **Do not suppress warnings.** Never add `#[allow(...)]` attributes to silence clippy lints. Fix the underlying code instead.
- **Do not make functional changes to fix CI.** Formatting and lint fixes must be purely structural — rename, reorder, collapse, derive — without altering runtime behavior.
