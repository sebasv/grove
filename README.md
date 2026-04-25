# grove

A terminal UI for cultivating git repositories, worktrees, and the work inside them.

Grove keeps all your active branches in one view — navigate between worktrees in the sidebar, run commands in embedded terminals, and review diffs without ever leaving the TUI.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ PROJECTS                       ┃ [▶ Term 1] [Term 2] [+]   [Diff: local]     │
│ ────────────────────────────── ┃ ─────────────────────────────────────────── │
│ ▼ grove                        ┃                                             │
│   ○ main              ✓        ┃ ~/grove/feat-sidebar$ cargo test            │
│   ● feat/sidebar    +4 ↑1 ●    ┃    Compiling grove v1.3.0                   │
│   ● fix/deps        ~2 ⚠       ┃    Finished dev in 3.42s                    │
│                                ┃ running 42 tests ... ok                     │
│ ▼ dotfiles                     ┃                                             │
│   ○ main              ✓        ┃ ~/grove/feat-sidebar$ █                     │
│   ● wip/zsh         +1 ~3      ┃                                             │
│                                ┃                                             │
│ ─────────────────────────────  ┃                                             │
│ ⟳ fetching origin (grove)      ┃                                             │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Features

- Sidebar with live git status badges per worktree (staged / modified / ahead / behind / conflicts; see [Status badges](#status-badges))
- Per-worktree "thinking" / "needs attention" indicator so you can tell at a glance which shells are busy or waiting on input
- Embedded PTY terminals — one or more per worktree, kept alive in the background
- Local diff view (staged + unstaged) and branch diff view (`<base>...HEAD`)
- GitHub PR and CI status polling shown alongside the git badges
- Background `git fetch` per repo so branch lists are fresh when you need them
- Activity footer in the sidebar showing what's currently running, plus a warning when GitHub auth is missing
- Unified, filter-as-you-type modal for creating worktrees off any local or remote branch
- Auto-detected base branch (`origin/HEAD`) per repo, with a configurable fallback
- Mouse support — click to focus, scroll wheel in terminals and diffs
- Persistent UI state including theme — reopens exactly where you left off
- Three built-in themes (`default`, `tokyonight`, `gruvbox`) — cycle with `F2`
- Zero dependencies — single binary, no tmux required

## Install

```sh
brew tap sebasv/grove
brew install grove
```

Or download a binary directly from the [releases page](https://github.com/sebasv/grove/releases).

## Quickstart

```sh
grove                     # open with whatever you've already configured
grove .                   # treat the current dir as the target
grove ~/dev/my-repo       # target a single repo (added to config if new)
grove ~/dev               # scan one level down for repos and pick which to add
```

When you point grove at a directory containing several repos, it shows a confirmation modal listing every repo it found — toggle them on/off and confirm to add the selected ones.

On first launch with no path, the sidebar is empty. Press `a` to add a repository — type or tab-complete the path, hit `Enter`. Repeat for each repo you want grove to track.

That's it. Worktrees inside each repo are discovered automatically, and your choices are persisted in `config.toml` for next time.

### Key bindings

| Key | Action |
|-----|--------|
| `j` / `k` | Move cursor in sidebar |
| `h` / `l` | Collapse / expand repo |
| `Enter` | Activate worktree |
| `a` / `R` | Add / remove repository |
| `w` / `W` | New / remove worktree |
| `Ctrl+T` / `Ctrl+W` | New / close terminal tab |
| `Ctrl+H` / `Ctrl+L` | Previous / next terminal tab |
| `Ctrl+Space` | Cycle focus: sidebar ↔ main pane |
| `Ctrl+\` | Toggle terminal scrollback mode |
| `Ctrl+D` | Toggle diff view |
| `m` | Toggle diff mode (local ↔ branch) |
| `r` | Refresh statuses and kick off a fetch for every repo |
| `F2` | Cycle theme (`default` → `tokyonight` → `gruvbox`) |
| `?` | Help overlay |
| `q` / `Ctrl+C` | Quit |

See `?` inside grove for the full reference.

### Status badges

Each worktree row carries up to two clusters of badges: git working-tree state (left) and GitHub PR + CI state (right).

**Working tree.** Shown left-to-right by priority — most urgent first.

| Badge | Meaning |
|-------|---------|
| `✓` (green) | Clean working tree, in sync with upstream |
| `⚠` (red) | Merge conflicts present (always shown first) |
| `+N` (green) | `N` files staged for commit |
| `~N` (yellow) | `N` files modified but not staged |
| `-N` (red) | `N` files deleted |
| `↑N` (dim) | `N` commits ahead of upstream |
| `↓N` (dim) | `N` commits behind upstream |

**Agent activity.** Grove watches each shell's window-title and bell signals to surface what's happening per worktree:

| Badge | Meaning |
|-------|---------|
| `!` (warn) | A shell rang the terminal bell (BEL) — likely waiting for input. Cleared when you focus the worktree. |
| `…` (dim) | Shell signalled "thinking": its window title starts with a braille spinner glyph (Claude Code, `gum`, oh-my-zsh prompts), or — for shells that don't set a title — it wrote PTY output in the last few seconds. |
| _(none)_ | Idle |

The thinking signal is precise for any TUI that uses an OSC 0/2 spinner — for Claude Code that's the `⠂ <prompt>` cycle while streaming, switching to `✳ <prompt>` once done. Bell is the high-confidence "needs attention" indicator since most readline-based prompts ring it.

**Pull request and CI.** Pulled from GitHub when grove can find a token (`GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth login`). When unauthenticated and at least one repo has a GitHub remote, the sidebar footer shows `⚠ PRs: not authenticated`.

| Badge | Meaning |
|-------|---------|
| `●` (green) | Open PR |
| `◐` (yellow) | Draft PR |
| `✓●` (dim green) | PR merged |
| `●` (dim) | PR closed without merging |
| `✗` (red) | CI failing on the PR |
| `⟳` (yellow) | CI pending on the PR |

CI passing or unconfigured is intentionally silent — no glyph after the PR badge means everything is fine.

### Activity footer

The bottom of the sidebar shows what grove is doing in the background — fetches, status refreshes, PR polls, scans. Idle = blank. The same row carries the GitHub-auth warning when applicable. This is the answer to "why did that badge just change?".

### Power-user config

Grove reads an optional TOML config for defaults and per-repo overrides. Run `grove --print-paths` to see where grove stores config, state, and logs. Edits are picked up next launch; syntax errors fall back to defaults rather than refusing to start (the error is logged).

```toml
# macOS: ~/Library/Application Support/grove/config.toml
# Linux: ~/.config/grove/config.toml

[general]
default_base_branch = "main"      # fallback when origin/HEAD isn't set
# worktree_root = "~/worktrees"   # optional; default is next to the repo
fetch_cadence_secs = 300          # background fetch interval per repo (0 disables)

[theme]
base = "default"                  # default | tokyonight | gruvbox

[[repos]]
name = "my-project"
path = "/Users/you/dev/my-project"
base_branch = "master"            # optional per-repo override
# worktree_root = "/work/my-proj" # optional per-repo override
```

## Feedback and contributions

Grove is in active development. If something doesn't work, feels off, or you have an idea — please [open an issue](https://github.com/sebasv/grove/issues). Pull requests are welcome too; check the issues list for anything tagged `good first issue` or just reach out before starting something large.

## License

MIT
