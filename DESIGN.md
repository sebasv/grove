# Grove — Design Document

## Concept

A terminal UI application for managing git repositories and worktrees. Inspired by Apache Superset's multi-pane dashboard model, but purpose-built for developer workflow inside a terminal.

Core value: see all your active branches/worktrees at a glance, work in any of them in embedded terminals, and review diffs — without leaving the app.

**Resolved design decisions:**
- Turn-key — zero-setup, no pre-existing tmux/session/config required.
- Terminals → embedded PTYs rendered as ratatui widgets via `tui-term`.
- PR provider → GitHub only.
- Diff modes → two: *local changes* (staged + unstaged) and *branch diff* (worktree vs. main).
- Session persistence → deferred to v1.5 (optional tmux backing).

---

## Layout

```
┌─ Grove ─────────────────────────────────────────────────────────────────┐
│                                                                             │
│  PROJECTS                  ┃  [▶ Term 1] [Term 2] [+]  ╱  [Diff: local]   │
│  ─────────────────────     ┃ ───────────────────────────────────────────── │
│  ▼ grove                ┃                                                │
│    ○ main          ✓       ┃  ~/grove/feat-sidebar$ cargo test          │
│    ● feat/sidebar  +4 ↑1   ┃     Compiling grove v0.1.0                 │
│    ● fix/deps      ~2 ⚠    ┃     Finished dev [unoptimized] in 3.42s       │
│                            ┃  running 42 tests                             │
│  ▼ dotfiles                ┃  test sidebar::renders_badges ... ok          │
│    ○ main          ✓       ┃  test diff::groups_by_file ... ok             │
│    ● wip/zsh       +1 ~3   ┃  ...                                          │
│                            ┃  test result: ok. 42 passed; 0 failed         │
│  ─────────────────────     ┃                                                │
│  [a] Add repo              ┃  ~/grove/feat-sidebar$ █                   │
│  [w] New worktree          ┃                                                │
│                            ┃                                                │
│  j/k  navigate             ┃                                                │
│  ?    help                 ┗───────────────────────────────────────────── │
└─────────────────────────────────────────────────────────────────────────────┘
```

Each terminal tab is a live PTY rendered in-pane. Selecting a worktree in the sidebar swaps the main pane to that worktree's terminals. Terminals keep running when you navigate away — they're suspended visually, not killed.

### Diff view — local changes mode

Shows `git diff` (unstaged) and `git diff --staged` combined, grouped by file.

```
│  [Term 1] [Term 2] [+]  ╱  [▶ Diff: local ▾]                             │
│ ──────────────────────────────────────────────────────────────────────── │
│  FILES                  ┃  src/sidebar.rs  (+8 -3)   [unstaged]           │
│  ────────────────────   ┃ ──────────────────────────────────────────────  │
│  UNSTAGED               ┃   28   │  fn render_item(                       │
│  ▶ src/sidebar.rs +8-3  ┃   29   │      item: &WorktreeItem,             │
│    src/app.rs     ~1    ┃   30 - │      color: Color,                    │
│                         ┃   30 + │      badge: &StatusBadge,             │
│  STAGED                 ┃   31   │  ) -> Line {                          │
│    Cargo.toml     +2    ┃   32 - │      Color::White                     │
│                         ┃   32 + │      badge.color()                    │
│                         ┃   33   │  }                                    │
│  [s] Stage  [u] Unstage ┃                                                 │
```

### Diff view — branch diff mode

Shows `git diff main...HEAD` — everything that diverged from main on this branch.

```
│  [Term 1] [Term 2] [+]  ╱  [▶ Diff: branch ▾]                            │
│ ──────────────────────────────────────────────────────────────────────── │
│  FILES (vs main)        ┃  src/sidebar.rs  (+34 -12)                      │
│  ────────────────────   ┃ ──────────────────────────────────────────────  │
│  src/sidebar.rs +34-12  ┃   ...full branch diff for this file...          │
│  src/app.rs     +6 -2   ┃                                                 │
│  Cargo.toml     +3      ┃                                                 │
│                         ┃                                                 │
│  3 files  +43 -14       ┃                                                 │
│  base: main             ┃                                                 │
```

---

## Embedded Terminals

### Stack

```
┌─────────────────────────────────────────────┐
│ ratatui widget tree                         │
│  └── PtyTerminal (wraps tui-term)           │
│       ├── grid state   (vt100::Parser)      │
│       └── input forwarder                    │
├─────────────────────────────────────────────┤
│ portable-pty                                 │
│  └── child process (shell)                   │
│       cwd = <worktree path>                  │
└─────────────────────────────────────────────┘
```

### Lifecycle per terminal

1. **Spawn.** On `T` (new terminal):
   - `portable_pty::PtySystem::openpty(size)` → master/slave file descriptors
   - Spawn `$SHELL -l` on the slave with `cwd = worktree.path`
   - Store master fd + child pid in `Terminal` struct
2. **Read.** A dedicated `tokio::task` reads from the PTY master and feeds bytes into a `vt100::Parser`. On each read, emit `Event::TerminalOutput(TerminalId)` to trigger a repaint (coalesced at 60 fps max).
3. **Render.** In the render pass, `tui-term::PseudoTerminal::new(parser.screen())` produces a widget that ratatui draws into the assigned rect.
4. **Input.** When the terminal pane has focus, ratatui key events are translated to byte sequences (`\r`, `\x1b[A`, etc.) and written to the PTY master.
5. **Resize.** On pane resize, call `pty_master.resize(PtySize { rows, cols, .. })` — the shell gets SIGWINCH automatically.
6. **Kill.** Drop the master fd; send SIGHUP to the child if it's still alive.

### Scrollback mode

The terminal pane has two modes:

| Mode | Behavior |
|------|----------|
| **Insert** (default) | Keys go to PTY; `Ctrl+[` enters scrollback |
| **Scrollback** | `j/k`, `PgUp`/`PgDn`, `g/G` navigate vt100's history buffer; `Esc` or `i` returns to Insert |

Distinct visual indicator in the tab bar: `[Term 1]` vs. `[Term 1 ⇅]`.

### Terminal tabs

Each worktree has its own `Vec<Terminal>`. The tab bar above the content pane shows only the active worktree's terminals. Switching worktrees in the sidebar swaps the full terminal list — previous terminals keep running in the background.

Tab bar keys (content pane focused, Insert mode):

| Key | Action |
|-----|--------|
| `Ctrl+T` | New terminal for current worktree |
| `Ctrl+W` | Kill current terminal (confirm) |
| `Ctrl+→` / `Ctrl+←` | Next / prev terminal |
| `Ctrl+[` | Enter scrollback mode |
| `Ctrl+D` | Pass through to shell (don't intercept) |

**Key passthrough rule:** in Insert mode, all keys except the `Ctrl+T / Ctrl+W / Ctrl+← / Ctrl+→ / Ctrl+[` set are forwarded to the PTY. This means `Tab` inside a shell doesn't switch focus zones — the user has to press `Esc` first, or we pick a less-collidey focus-cycle key like `F2`. (See open question below.)

### Resource budget

Per terminal: ~1 MB baseline (parser grid + scrollback buffer of ~10k lines) + shell memory. Ten terminals ≈ 10 MB grove overhead plus shell memory. Fine.

---

## Session Persistence — Deferred to v1.5

v1.0 embedded PTYs do not survive grove exit. Users close the app → shells die.

**v1.5 plan:** optional tmux backing. Instead of spawning a shell directly in the PTY, spawn `tmux new-session -A -D -s grove-<uuid>`. The shell runs inside tmux; tmux survives grove's death; on relaunch, grove reattaches to the existing tmux session. This is invisible to the user — tmux's status bar is hidden (`tmux set status off`) and the session ID is tracked in grove's state file.

Requires tmux on the system — grove detects on startup and prompts `tmux not found. Install with 'brew install tmux' to enable session persistence. Continue without? (y/N)`. App is fully usable without tmux; persistence is the upgrade.

Deferred because: it adds a runtime dependency detection path, a state file for tmux session tracking, and reattach logic. None of that is needed to validate the core UX.

---

## Status Badges

Each worktree row in the sidebar shows compact status glyphs:

| Glyph | Meaning |
|-------|---------|
| `✓` | Clean, no uncommitted changes |
| `+N` | N staged files |
| `~N` | N modified (unstaged) files |
| `-N` | N deleted files |
| `↑N` | N commits ahead of remote |
| `↓N` | N commits behind remote |
| `⚠` | Merge conflict |
| `●` | GitHub PR open |
| `◐` | GitHub PR draft |
| `✓●` | PR merged |
| `✗` | CI checks failing |
| `⟳` | CI checks running |

Colors: green = clean, yellow = modified/ahead, red = conflict/CI fail, dim = no remote/behind.

Badge layout per row (left to right, omit zero-count items):

```
  ● feat/sidebar    +4 ~1 ↑2  ⟳
  ● fix/deps        ~2 ⚠
  ○ main            ✓
```

---

## Navigation Model

Three focus zones, cycled with `Ctrl+Space` (avoiding `Tab` because shells use it). Keybindings are hardcoded in v1.0; configurability planned for a later release.

1. **Sidebar** — repo/worktree tree
2. **Tab bar** — terminal tabs or diff tab + mode selector
3. **Content pane** — terminal or diff viewer

### Sidebar keys

| Key | Action |
|-----|--------|
| `j` / `k` | Move up/down |
| `h` / `l` or `←` / `→` | Collapse / expand repo |
| `Enter` | Activate worktree in main pane |
| `w` | Create new worktree (modal) |
| `W` | Delete focused worktree (confirmation) |
| `a` | Add repository (path prompt) |
| `R` | Remove repository (confirmation) |
| `r` | Refresh git status |
| `p` | Open GitHub PR in browser |

### Terminal pane — Insert mode

| Key | Action |
|-----|--------|
| `Ctrl+T` | New terminal |
| `Ctrl+W` | Kill current terminal (confirm) |
| `Ctrl+→` / `Ctrl+←` | Next / prev terminal |
| `Ctrl+[` | Enter scrollback mode |
| `Ctrl+Space` | Shift focus to sidebar/tab bar |
| all else | Forwarded to shell |

### Terminal pane — Scrollback mode

| Key | Action |
|-----|--------|
| `j` / `k` / arrows | Scroll one line |
| `PgUp` / `PgDn` | Scroll one page |
| `g` / `G` | Top / bottom |
| `/` | Search in scrollback |
| `Esc` / `i` | Back to Insert mode |

### Diff pane

| Key | Action |
|-----|--------|
| `j` / `k` | Next / prev file in file list |
| `Tab` | Toggle focus: file list ↔ diff content |
| `J` / `K` | Scroll diff content down/up |
| `s` | Stage file (local mode only) |
| `u` | Unstage file (local mode only) |
| `m` | Switch diff mode: local ↔ branch |

### Global

| Key | Action |
|-----|--------|
| `?` | Help overlay |
| `q` (sidebar/diff focus only) | Quit (confirm if unsaved diffs or running processes) |
| `Ctrl+R` | Force refresh git + PR status |

---

## Architecture

### State model

```
AppState
├── repos: Vec<Repo>
│   ├── id: RepoId
│   ├── name: String
│   ├── root_path: PathBuf
│   └── worktrees: Vec<Worktree>
│       ├── id: WorktreeId
│       ├── branch: String
│       ├── path: PathBuf
│       ├── git_status: GitStatus         ← refreshed on .git change
│       │   ├── staged: Vec<FileStatus>
│       │   ├── unstaged: Vec<FileStatus>
│       │   ├── conflicts: Vec<PathBuf>
│       │   ├── ahead: u32
│       │   └── behind: u32
│       ├── pr_status: Option<PrStatus>   ← polled from GitHub every 30s
│       ├── terminals: Vec<Terminal>      ← live PTYs (see below)
│       └── diff: DiffState               ← lazy-loaded
│           ├── mode: DiffMode            (Local | Branch)
│           ├── files: Vec<DiffFile>
│           └── active_file: usize
│
├── ui: UiState
│   ├── sidebar_focus: SidebarItem
│   ├── active_worktree: Option<WorktreeId>
│   ├── main_tab: MainTab                 (Terminals | Diff)
│   ├── active_terminal: usize            ← per-worktree index
│   ├── diff_mode: DiffMode
│   ├── focus_zone: FocusZone
│   ├── terminal_mode: TerminalMode       (Insert | Scrollback)
│   └── modal: Option<Modal>
│
└── config: Config
    ├── repos: Vec<RepoConfig>
    ├── github_token: Option<String>
    ├── default_base_branch: String
    ├── shell: Option<String>             ← defaults to $SHELL
    └── theme: Theme
```

```
Terminal
├── id: TerminalId
├── pty_master: Arc<Mutex<dyn MasterPty>> ← portable-pty
├── child: Arc<Mutex<dyn Child>>
├── parser: Arc<Mutex<vt100::Parser>>     ← grid + scrollback
├── title: String                         ← updates via OSC 0/2 escape codes
└── created_at: Instant
```

### Component tree

```
App
├── Sidebar
│   ├── RepoGroup[] (collapsible)
│   │   └── WorktreeRow (branch + badges)
│   └── SidebarFooter
├── MainPanel
│   ├── TabBar (terminal tabs + diff toggle + mode selector)
│   ├── TerminalPane
│   │   └── PtyTerminalWidget (tui-term-backed)
│   └── DiffPane
│       ├── DiffFileList
│       └── DiffContent
└── Modal (overlay, optional)
    ├── NewWorktreeModal
    ├── AddRepoModal
    ├── ConfirmModal
    └── HelpOverlay
```

### Background tasks

| Task | Trigger | Produces |
|------|---------|---------|
| `git_watcher` | `notify` on `<worktree>/.git` | `Event::GitStatusChanged(WorktreeId)` |
| `github_poller` | 30 s per worktree with a branch | `Event::PrStatusChanged(WorktreeId, PrStatus)` |
| `pty_reader` | one per terminal, reads master fd | `Event::TerminalOutput(TerminalId)` |
| `diff_loader` | worktree activation / mode switch | `Event::DiffLoaded(WorktreeId, DiffMode, DiffState)` |

All events flow through a single `mpsc::channel` to the UI thread. The UI coalesces `TerminalOutput` events at 60 Hz — raw PTY bytes arrive much faster than necessary for rendering.

---

## Diff Modes — Implementation

### Local changes (`DiffMode::Local`)

```
unstaged = repo.diff_index_to_workdir(None, None)
staged   = repo.diff_tree_to_index(HEAD_tree, None, None)
```

File list groups unstaged first, then staged. A file can appear in both groups if it has both staged and unstaged changes.

### Branch diff (`DiffMode::Branch`)

```
merge_base = repo.merge_base(HEAD, base_commit)
base_tree  = repo.find_commit(merge_base).tree()
diff       = repo.diff_tree_to_tree(base_tree, HEAD_tree, None)
```

Three-dot merge base so we only show this branch's changes, not divergence on main. Base is configurable per repo; default `main`.

---

## Technology Stack

| Concern | Choice |
|---------|--------|
| TUI framework | [`ratatui`](https://github.com/ratatui-org/ratatui) |
| Terminal backend | `crossterm` |
| PTY | [`portable-pty`](https://github.com/wez/wezterm/tree/main/pty) |
| Terminal emulator | [`vt100`](https://github.com/doy/vt100-rust) |
| Embedded term widget | [`tui-term`](https://github.com/a-kenji/tui-term) |
| Git | [`git2`](https://github.com/rust-lang/git2-rs) (libgit2) |
| GitHub | [`octocrab`](https://github.com/XAMPPRocky/octocrab) |
| Async runtime | `tokio` |
| FS watching | [`notify`](https://github.com/notify-rs/notify) |
| Syntax highlight | [`syntect`](https://github.com/trishume/syntect) |
| Config | `serde` + TOML |

---

## Configuration (`~/.config/grove/config.toml`)

```toml
[general]
default_base_branch = "main"
shell = "/bin/zsh"               # optional; defaults to $SHELL

[github]
# token read from GITHUB_TOKEN env if not set

[theme]
base = "tokyonight"

[[repos]]
name = "grove"
path = "/Users/sebas/dev/grove"
base_branch = "main"

[[repos]]
name = "dotfiles"
path = "/Users/sebas/dotfiles"
```

Worktrees discovered automatically via `git worktree list --porcelain`.

---

## Open Questions

1. **True color / font support.** vt100 supports 24-bit color, but terminal hosts with limited color palettes may misrender. Trust the outer terminal.

---

## Phased Delivery

| Phase | Scope |
|-------|-------|
| **v0.1 — skeleton** | Sidebar with static repo list, basic `j/k` navigation |
| **v0.2 — git status** | Live badges via `git2` + `notify` watcher |
| **v0.3 — embedded terminal (one)** | Single PTY in main pane via `tui-term`; Insert/Scrollback modes |
| **v0.4 — terminal tabs** | Multiple PTYs per worktree; tab bar |
| **v0.5 — diff: local** | Local changes viewer, stage/unstage |
| **v0.6 — diff: branch** | Branch-vs-main mode, mode switcher |
| **v0.7 — GitHub PR** | PR + CI badge polling via `octocrab` |
| **v0.8 — worktree management** | Create/delete worktrees from TUI |
| **v1.0** | Config file, themes, stable keybindings, `--help` |
| **v1.1** | Mouse support — click to focus panes, scroll wheel in terminal/diff, click tabs |
| **v1.2** | User-configurable keybindings |
| **v1.5** | Optional tmux backing for session persistence |
