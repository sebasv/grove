# grove

A terminal UI for cultivating git repositories, worktrees, and the work inside them.

Grove keeps all your active branches in one view — navigate between worktrees in the sidebar, run commands in embedded terminals, and review diffs without ever leaving the TUI.

```
┌─ Grove ─────────────────────────────────────────────────────────────────┐
│                                                                          │
│  PROJECTS                  ┃  [▶ Term 1] [Term 2] [+]  ╱  [Diff: local] │
│  ─────────────────────     ┃ ─────────────────────────────────────────── │
│  ▼ grove                   ┃                                             │
│    ○ main          ✓       ┃  ~/grove/feat-sidebar$ cargo test           │
│    ● feat/sidebar  +4 ↑1   ┃     Compiling grove v1.1.0                 │
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
- Zero dependencies — single binary, no tmux required

## Install

```sh
brew tap sebasv/grove
brew install grove
```

Or download a binary directly from the [releases page](https://github.com/sebasv/grove/releases).

## Quickstart

On first launch, grove looks for a config file and tells you where to create it if it is missing. The config is a simple TOML file:

```toml
# macOS: ~/Library/Application Support/grove/config.toml
# Linux: ~/.config/grove/config.toml

[[repos]]
name = "my-project"
path = "/Users/you/dev/my-project"

[[repos]]
name = "dotfiles"
path = "/Users/you/dotfiles"
base_branch = "master"   # optional, defaults to "main"
```

Then just run:

```sh
grove
```

### Key bindings

| Key | Action |
|-----|--------|
| `j` / `k` | Move cursor in sidebar |
| `h` / `l` | Collapse / expand repo |
| `Enter` | Activate worktree |
| `Ctrl+T` | New terminal for current worktree |
| `Ctrl+Space` | Cycle focus: sidebar → tab bar → content pane |
| `m` | Toggle diff mode (local ↔ branch) |
| `?` | Help overlay |
| `q` | Quit |

See `?` inside grove for the full reference.

## Feedback and contributions

Grove is in active development. If something doesn't work, feels off, or you have an idea — please [open an issue](https://github.com/sebasv/grove/issues). Pull requests are welcome too; check the issues list for anything tagged `good first issue` or just reach out before starting something large.

## License

MIT
