# Agent Directives

## Working on PRs

When the current branch has an open PR, all changes must end up on a pushed commit — never leave work as uncommitted edits or unpushed commits, since reviewers and CI only see what's on the remote.

Any work on a PR is not complete until CI passes. Before considering a PR done:

- Run `cargo fmt --check` and fix any formatting issues with `cargo fmt`.
- Run `cargo clippy -- -D warnings` and fix all warnings without using `#[allow(...)]`.
- Run `cargo test` and ensure all tests pass.
- Push the fixes and confirm CI is green before marking work as complete or merging.

## CI Fixes

When fixing CI failures (clippy, fmt, tests):

- **Do not suppress warnings.** Never add `#[allow(...)]` attributes to silence clippy lints. Fix the underlying code instead.
- **Do not make functional changes to fix CI.** Formatting and lint fixes must be purely structural — rename, reorder, collapse, derive — without altering runtime behavior.

## TUI Changes

When changing anything that renders to the terminal (modals, sidebar rows, status lines, hints, error messages):

- **Verify the change does not overflow its container.** Modals are fixed-width; long strings written with a single-line `Paragraph` will spill past the right border and look broken. Either keep the text within the inner width, or render with `Wrap { trim: false }` and allocate enough rows for the wrapped result.
- **Snapshot the rendering.** Add or update an `insta` snapshot in `src/ui/snapshots/` that exercises the new content (especially error paths) so future changes that re-introduce overflow are visible in review.
