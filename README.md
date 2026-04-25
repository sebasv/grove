# grove

A terminal UI for cultivating git repositories, worktrees, and the work inside them.

If you juggle several feature branches at once and find `git worktree add` painful to drive by hand, grove keeps all of them in one place: navigate between worktrees in the sidebar, run commands in embedded terminals, and review diffs without ever leaving the TUI.

```
┌─ Grove ─────────────────────────────────────────────────────────────────┐
│                                                                          │
│  PROJECTS                  ┃  [▶ Term 1] [Term 2] [+]  ╱  [Diff: local] │
│  ─────────────────────     ┃ ─────────────────────────────────────────── │
│  ▼ grove                   ┃                                             │
│    ○ main          ✓       ┃  ~/grove/feat-sidebar$ cargo test           │
│    ● feat/sidebar  +4 ↑1   ┃     Compiling grove                         │
│    ● fix/deps      ~2 ⚠    ┃     Finished dev in 3.42s                  │
│                            ┃  running 42 tests ... ok                    │
│  ▼ dotfiles                ┃                                             │
│    ○ main          ✓       ┃  ~/grove/feat-sidebar$ █                   │
│    ● wip/zsh       +1 ~3   ┃                                             │
└─────────────────────────────────────────────────────────────────────────┘
```

## Features

- Sidebar with live git status badges per worktree (`+N staged`, `~N modified`, `↑N ahead`, `⚠ conflict`, …)
- Embedded PTY terminals — one or more per worktree, keep running in the background
- Local diff view (staged + unstaged) and branch diff view (`main...HEAD`)
- GitHub PR and CI status polling
- Create and delete worktrees without leaving the app
- Mouse support — click to focus, scroll wheel in terminals and diffs
- Persisted UI state — reopens exactly where you left off
- Single binary — no tmux, no system services, just `git` on your `PATH`

## Install

Homebrew (macOS and Linux):

```sh
brew tap sebasv/grove
brew install grove
```

Or download a prebuilt binary for your platform from the [releases page](https://github.com/sebasv/grove/releases).

## Quickstart

```sh
grove
```

On first launch the sidebar is empty. Press `a` to add a repository — type or tab-complete the path, hit `Enter`. Repeat for each repo you want grove to track.

That's it. Worktrees inside each repo are discovered automatically, and your choices are persisted in `config.toml` for next time.

### Key bindings

| Key | Action |
|-----|--------|
| `j` / `k` | Move cursor in sidebar |
| `h` / `l` | Collapse / expand repo |
| `Enter` | Activate worktree |
| `a` / `R` | Add / remove repository |
| `w` / `W` | New / remove worktree |
| `Ctrl+T` | New terminal for current worktree |
| `Ctrl+Space` | Cycle focus: sidebar → tab bar → content pane |
| `m` | Toggle diff mode (local ↔ branch) |
| `?` | Help overlay |
| `q` / `Ctrl+C` | Quit |

See `?` inside grove for the full reference.

### GitHub integration

PR and CI status badges only appear when grove can authenticate to GitHub. It looks for credentials in this order:

1. `GITHUB_TOKEN` or `GH_TOKEN` environment variable.
2. `gh auth token` — works after `gh auth login` (including the SSH-key flow).

Without either, grove still runs; the PR and CI badges just stay blank.

### Power-user config

Grove reads an optional TOML config for defaults and per-repo overrides. Run `grove --print-paths` to see where grove stores config, state, and logs.

```toml
# macOS: ~/Library/Application Support/grove/config.toml
# Linux: ~/.config/grove/config.toml

[general]
default_base_branch = "main"        # fallback when origin/HEAD isn't set
# worktree_root = "~/worktrees"     # optional; default is next to the repo

[[repos]]
name = "my-project"
path = "/Users/you/dev/my-project"
base_branch = "master"              # optional per-repo override
```

## Feedback and contributions

Grove is stable and evolving. If something doesn't work, feels off, or you have an idea — please [open an issue](https://github.com/sebasv/grove/issues). Pull requests are welcome too; check the issues list for anything tagged `good first issue` or just reach out before starting something large. [`DESIGN.md`](DESIGN.md) is the place to start if you want context on how grove is put together before opening a PR.

## License

MIT
