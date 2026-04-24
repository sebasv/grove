use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{BranchType, DiffOptions, Repository, Status, StatusOptions};

use crate::model::{Worktree, WorktreeStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: PathBuf,
    pub staged: bool,
    pub adds: u32,
    pub dels: u32,
    pub kind: DeltaKind,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
}

pub fn compute_branch_diff(worktree_path: &Path, base_branch: &str) -> Result<Vec<DiffFile>> {
    let repo = Repository::open(worktree_path)
        .with_context(|| format!("opening {}", worktree_path.display()))?;
    let head = repo.head().context("resolving HEAD")?;
    let head_oid = head
        .target()
        .ok_or_else(|| anyhow::anyhow!("HEAD has no OID"))?;

    // Prefer origin/<base> so the diff reflects the remote tip rather than a
    // potentially stale local branch (common in worktree workflows where the
    // local base branch is never checked out or pulled).
    let remote_name = format!("origin/{base_branch}");
    let base_oid = if let Ok(b) = repo.find_branch(&remote_name, BranchType::Remote) {
        match b.get().target() {
            Some(oid) => oid,
            None => return Ok(Vec::new()),
        }
    } else {
        match repo.find_branch(base_branch, BranchType::Local) {
            Ok(b) => match b.get().target() {
                Some(oid) => oid,
                None => return Ok(Vec::new()),
            },
            Err(_) => return Ok(Vec::new()),
        }
    };

    let merge_base = match repo.merge_base(head_oid, base_oid) {
        Ok(oid) => oid,
        Err(_) => return Ok(Vec::new()),
    };
    let base_tree = repo.find_commit(merge_base)?.tree()?;
    let head_tree = repo.find_commit(head_oid)?.tree()?;

    let mut opts = DiffOptions::new();
    opts.context_lines(3);
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))
        .context("diff tree to tree")?;

    let mut files = Vec::new();
    // Branch diffs are conceptually unstaged for our UI (all are "changes on
    // this branch vs base"), but the "staged" flag isn't meaningful here —
    // we set it to false so the file list renders with the modified glyph.
    collect_diff(&diff, false, &mut files);
    Ok(files)
}

pub fn compute_local_diff(worktree_path: &Path) -> Result<Vec<DiffFile>> {
    let repo = Repository::open(worktree_path)
        .with_context(|| format!("opening {}", worktree_path.display()))?;

    let mut opts = DiffOptions::new();
    opts.context_lines(3);

    let mut files = Vec::new();

    let unstaged = repo
        .diff_index_to_workdir(None, Some(&mut opts))
        .context("diff index to workdir")?;
    collect_diff(&unstaged, false, &mut files);

    let head_tree = repo.head().and_then(|h| h.peel_to_tree()).ok();
    if let Some(tree) = head_tree {
        let mut opts = DiffOptions::new();
        opts.context_lines(3);
        let staged = repo
            .diff_tree_to_index(Some(&tree), None, Some(&mut opts))
            .context("diff tree to index")?;
        collect_diff(&staged, true, &mut files);
    }

    Ok(files)
}

fn collect_diff(diff: &git2::Diff<'_>, staged: bool, out: &mut Vec<DiffFile>) {
    use std::cell::RefCell;

    let scratch: RefCell<Vec<DiffFile>> = RefCell::new(Vec::new());
    let _ = diff.foreach(
        &mut |delta, _progress| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let kind = match delta.status() {
                git2::Delta::Added => DeltaKind::Added,
                git2::Delta::Modified => DeltaKind::Modified,
                git2::Delta::Deleted => DeltaKind::Deleted,
                git2::Delta::Renamed => DeltaKind::Renamed,
                _ => DeltaKind::Other,
            };
            scratch.borrow_mut().push(DiffFile {
                path,
                staged,
                adds: 0,
                dels: 0,
                kind,
                hunks: Vec::new(),
            });
            true
        },
        None,
        Some(&mut |_delta, hunk| {
            if let Some(file) = scratch.borrow_mut().last_mut() {
                let header = std::str::from_utf8(hunk.header()).unwrap_or("").to_string();
                file.hunks.push(DiffHunk {
                    header,
                    lines: Vec::new(),
                });
            }
            true
        }),
        Some(&mut |_delta, _hunk, line| {
            let kind = match line.origin() {
                '+' => DiffLineKind::Add,
                '-' => DiffLineKind::Del,
                _ => DiffLineKind::Context,
            };
            if let Some(file) = scratch.borrow_mut().last_mut() {
                if let Some(hunk) = file.hunks.last_mut() {
                    let content = std::str::from_utf8(line.content())
                        .unwrap_or("")
                        .to_string();
                    hunk.lines.push(DiffLine {
                        kind,
                        content: content.trim_end_matches('\n').to_string(),
                    });
                    match kind {
                        DiffLineKind::Add => file.adds += 1,
                        DiffLineKind::Del => file.dels += 1,
                        DiffLineKind::Context => {}
                    }
                }
            }
            true
        }),
    );
    out.extend(scratch.into_inner());
}

pub fn create_worktree(repo_root: &Path, branch: &str, path: &Path, base: &str) -> Result<()> {
    run_git_cmd(
        repo_root,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            &path.display().to_string(),
            base,
        ],
    )
}

pub fn remove_worktree(repo_root: &Path, worktree_path: &Path) -> Result<()> {
    run_git_cmd(
        repo_root,
        &["worktree", "remove", &worktree_path.display().to_string()],
    )
}

/// Derive a path for a new worktree.
///
/// When `worktree_root` is `Some(root)` the path is `<root>/<repo_name>/<branch>`.
/// When `None` the sibling strategy is used: `<repo_root_parent>/<repo_name>-<branch>`.
/// A numeric suffix is appended when the candidate path already exists.
pub fn derive_worktree_path(
    repo_root: &Path,
    repo_name: &str,
    branch: &str,
    worktree_root: Option<&Path>,
) -> PathBuf {
    let sanitised = branch.replace('/', "-");
    let (base_dir, stem) = match worktree_root {
        Some(root) => (root.join(repo_name), sanitised),
        None => {
            let parent = repo_root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            (parent, format!("{repo_name}-{sanitised}"))
        }
    };
    let mut candidate = base_dir.join(&stem);
    let mut counter = 2;
    while candidate.exists() {
        candidate = base_dir.join(format!("{stem}-{counter}"));
        counter += 1;
    }
    candidate
}

pub fn stage_path(worktree_path: &Path, file_path: &Path) -> Result<()> {
    run_git_cmd(
        worktree_path,
        &["add", "--", &file_path.display().to_string()],
    )
}

pub fn unstage_path(worktree_path: &Path, file_path: &Path) -> Result<()> {
    run_git_cmd(
        worktree_path,
        &[
            "restore",
            "--staged",
            "--",
            &file_path.display().to_string(),
        ],
    )
}

fn run_git_cmd(cwd: &Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub fn is_git_repo(path: &Path) -> bool {
    Repository::open(path).is_ok()
}

/// Resolve `origin/HEAD` to a branch name, e.g. `"main"` or `"master"`.
/// Returns `None` when the remote has never been fetched, `origin/HEAD`
/// was not set by the remote, or the repo has no `origin` at all.
///
/// Reads `refs/remotes/origin/HEAD` via libgit2 — no network access.
pub fn detect_default_branch(repo_root: &Path) -> Option<String> {
    let repo = Repository::open(repo_root).ok()?;
    let reference = repo.find_reference("refs/remotes/origin/HEAD").ok()?;
    // `origin/HEAD` is a symbolic reference pointing at something like
    // `refs/remotes/origin/main`.  The last path segment is the branch
    // name we want.
    let target = reference.symbolic_target()?.to_string();
    target.rsplit('/').next().map(str::to_string)
}

/// Walk `root` up to `depth` levels deep looking for git repositories.
/// Returns the absolute paths of each repo root in sorted order.
///
/// A "git repository" here is any directory that contains a `.git` entry
/// directly, or is a bare repository (has `HEAD` + `config` at the top
/// level).  Using a strict directory-local check — rather than libgit2's
/// upward-searching `Repository::open` — keeps the walker predictable
/// when a scan root lives inside another repo.
///
/// Skips hidden directories (except `.git` itself), and does not recurse
/// into directories named `node_modules`, `target`, `.venv` or similar
/// build / cache dirs to keep scans of large home directories fast.
pub fn discover_repos(root: &Path, depth: u8) -> Vec<PathBuf> {
    fn skip(name: &str) -> bool {
        matches!(
            name,
            "node_modules" | "target" | ".venv" | "venv" | "__pycache__" | "dist" | "build"
        )
    }

    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, level)) = stack.pop() {
        if looks_like_repo_root(&dir) {
            found.push(dir);
            continue; // don't descend into a repo
        }
        if level >= depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') && name != ".git" {
                continue;
            }
            if skip(name) {
                continue;
            }
            stack.push((path, level + 1));
        }
    }
    found.sort();
    found
}

fn looks_like_repo_root(path: &Path) -> bool {
    if path.join(".git").exists() {
        return true;
    }
    // Bare repositories have HEAD + config at the top level.
    path.join("HEAD").is_file() && path.join("config").is_file()
}

/// Is every commit on `branch` reachable from `base` (or its `origin/base`
/// tracking ref, when present)?  Returns `true` when a safe `git branch -d`
/// will succeed and `false` when a force-delete (`-D`) would be required.
///
/// Missing branches return `false` (treat the "branch doesn't exist" case
/// as unmerged — safer default for a deletion UI).  `origin/base` is
/// preferred over the local base because worktree workflows often leave
/// the local base branch behind.
pub fn is_branch_merged(repo_root: &Path, branch: &str, base: &str) -> bool {
    let Ok(repo) = Repository::open(repo_root) else {
        return false;
    };
    let Ok(branch_ref) = repo.find_branch(branch, BranchType::Local) else {
        return false;
    };
    let Some(branch_oid) = branch_ref.get().target() else {
        return false;
    };
    // Prefer origin/<base> for the same reason compute_branch_diff does:
    // in worktree workflows the local base branch is often never checked
    // out and may lag behind the remote.
    let base_oid = repo
        .find_branch(&format!("origin/{base}"), BranchType::Remote)
        .ok()
        .and_then(|b| b.get().target())
        .or_else(|| {
            repo.find_branch(base, BranchType::Local)
                .ok()
                .and_then(|b| b.get().target())
        });
    let Some(base_oid) = base_oid else {
        return false;
    };
    // "branch merged into base" iff every commit on branch is reachable
    // from base — equivalent to `git merge-base --is-ancestor branch base`.
    repo.graph_descendant_of(base_oid, branch_oid)
        .unwrap_or(false)
        || branch_oid == base_oid
}

pub fn list_worktrees(repo_root: &Path) -> Result<Vec<Worktree>> {
    let repo = Repository::open(repo_root)
        .with_context(|| format!("opening repository at {}", repo_root.display()))?;
    let mut out = Vec::new();

    // Primary checkout.
    if let Some(workdir) = repo.workdir() {
        // Fall back to an abbreviated OID for detached HEAD so the primary
        // checkout is never silently absent from the sidebar.
        let branch = branch_name_of_head(&repo).or_else(|| {
            repo.head().ok()?.target().map(|oid| {
                let s = oid.to_string();
                format!("(detached) {}", &s[..s.len().min(7)])
            })
        });
        if let Some(branch) = branch {
            out.push(Worktree {
                branch,
                path: workdir.to_path_buf(),
                is_primary: true,
                status: None,
                pr: None,
            });
        }
    }

    // Linked worktrees.
    let wt_names = repo.worktrees().context("listing linked worktrees")?;
    for name in wt_names.iter().flatten() {
        let Ok(wt) = repo.find_worktree(name) else {
            continue;
        };
        let wt_path = wt.path().to_path_buf();
        let Ok(wt_repo) = Repository::open_from_worktree(&wt) else {
            continue;
        };
        let Some(branch) = branch_name_of_head(&wt_repo) else {
            continue;
        };
        out.push(Worktree {
            branch,
            path: wt_path,
            is_primary: false,
            status: None,
            pr: None,
        });
    }

    // git enumerates .git/worktrees/ in filesystem (inode) order, which is not
    // stable across worktree creation. Sort linked worktrees alphabetically so
    // the sidebar order is deterministic regardless of when each was added.
    if out.len() > 1 {
        out[1..].sort_by(|a, b| a.branch.cmp(&b.branch));
    }

    Ok(out)
}

pub fn compute_status(worktree_path: &Path) -> Result<WorktreeStatus> {
    let repo = Repository::open(worktree_path)
        .with_context(|| format!("opening {}", worktree_path.display()))?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(false).include_ignored(false);

    let statuses = repo
        .statuses(Some(&mut opts))
        .context("computing statuses")?;

    let mut out = WorktreeStatus::default();
    for entry in statuses.iter() {
        let s = entry.status();
        if s.is_conflicted() {
            out.conflicts += 1;
            continue;
        }
        if s.intersects(
            Status::INDEX_NEW
                | Status::INDEX_MODIFIED
                | Status::INDEX_RENAMED
                | Status::INDEX_TYPECHANGE,
        ) {
            out.staged += 1;
        }
        if s.intersects(Status::WT_MODIFIED | Status::WT_RENAMED | Status::WT_TYPECHANGE) {
            out.modified += 1;
        }
        if s.intersects(Status::INDEX_DELETED | Status::WT_DELETED) {
            out.deleted += 1;
        }
    }

    let (ahead, behind) = ahead_behind(&repo).unwrap_or((0, 0));
    out.ahead = ahead as u32;
    out.behind = behind as u32;

    Ok(out)
}

fn branch_name_of_head(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().map(str::to_string)
}

fn ahead_behind(repo: &Repository) -> Result<(usize, usize)> {
    let head = repo.head()?;
    if !head.is_branch() {
        return Ok((0, 0));
    }
    let shorthand = head
        .shorthand()
        .ok_or_else(|| anyhow::anyhow!("HEAD has no shorthand"))?;
    let local = repo.find_branch(shorthand, BranchType::Local)?;
    let upstream = match local.upstream() {
        Ok(u) => u,
        Err(_) => return Ok((0, 0)),
    };
    let local_oid = local
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("no local OID"))?;
    let upstream_oid = upstream
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("no upstream OID"))?;
    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
    Ok((ahead, behind))
}

#[derive(Debug, Clone)]
pub struct BranchEntry {
    pub name: String,
    pub remote: Option<String>,
}

impl BranchEntry {
    pub fn display(&self) -> String {
        match &self.remote {
            None => self.name.clone(),
            Some(r) => format!("{r}/{}", self.name),
        }
    }

    pub fn is_remote_only(&self) -> bool {
        self.remote.is_some()
    }
}

pub fn list_branches(repo_root: &Path) -> Result<Vec<BranchEntry>> {
    let repo =
        Repository::open(repo_root).with_context(|| format!("opening {}", repo_root.display()))?;
    let mut out = Vec::new();
    let mut local_names = std::collections::HashSet::new();

    for b in repo.branches(Some(BranchType::Local))? {
        let (branch, _) = b?;
        if let Some(name) = branch.name()? {
            local_names.insert(name.to_string());
            out.push(BranchEntry {
                name: name.to_string(),
                remote: None,
            });
        }
    }

    for b in repo.branches(Some(BranchType::Remote))? {
        let (branch, _) = b?;
        if let Some(full) = branch.name()? {
            if let Some((remote, branch_name)) = full.split_once('/') {
                if branch_name != "HEAD" && !local_names.contains(branch_name) {
                    out.push(BranchEntry {
                        name: branch_name.to_string(),
                        remote: Some(remote.to_string()),
                    });
                }
            }
        }
    }

    Ok(out)
}

pub fn create_worktree_from_existing(repo_root: &Path, branch: &str, path: &Path) -> Result<()> {
    run_git_cmd(
        repo_root,
        &["worktree", "add", &path.display().to_string(), branch],
    )
}

pub fn create_worktree_from_remote(
    repo_root: &Path,
    remote: &str,
    branch_name: &str,
    local_name: &str,
    path: &Path,
) -> Result<()> {
    let remote_ref = format!("{remote}/{branch_name}");
    run_git_cmd(
        repo_root,
        &[
            "worktree",
            "add",
            "--track",
            "-b",
            local_name,
            &path.display().to_string(),
            &remote_ref,
        ],
    )
}

pub fn delete_branch(repo_root: &Path, branch: &str) -> Result<()> {
    run_git_cmd(repo_root, &["branch", "-d", branch])
}

pub fn force_delete_branch(repo_root: &Path, branch: &str) -> Result<()> {
    run_git_cmd(repo_root, &["branch", "-D", branch])
}

/// Shell out to `git fetch --all --prune` for `repo_root`.  Shelling
/// out (rather than calling libgit2) lets the user's `~/.gitconfig` —
/// `http.lowSpeedLimit`, proxy settings, SSH agent — drive the
/// network layer, so a repo that works on the command line also works
/// in grove.  The command inherits `stdout`/`stderr` from the caller
/// but we only surface them on failure.
pub fn fetch_remote(repo_root: &Path) -> Result<()> {
    run_git_cmd(repo_root, &["fetch", "--all", "--prune", "--quiet"])
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "grove-git-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_git_test(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_repo(dir: &Path) {
        run_git_test(dir, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        run_git_test(dir, &["add", "."]);
        run_git_test(
            dir,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
                "--quiet",
            ],
        );
    }

    #[test]
    fn is_git_repo_detects_real_repo() {
        let dir = temp_dir();
        init_repo(&dir);
        assert!(is_git_repo(&dir));
    }

    #[test]
    fn is_git_repo_rejects_non_repo() {
        let dir = temp_dir();
        assert!(!is_git_repo(&dir));
    }

    #[test]
    fn list_worktrees_returns_primary() {
        let dir = temp_dir();
        init_repo(&dir);
        let wts = list_worktrees(&dir).unwrap();
        assert_eq!(wts.len(), 1);
        assert!(wts[0].is_primary);
        assert_eq!(wts[0].branch, "main");
    }

    #[test]
    fn list_worktrees_includes_linked() {
        let dir = temp_dir();
        init_repo(&dir);
        let linked = dir.parent().unwrap().join(format!(
            "{}-linked",
            dir.file_name().unwrap().to_string_lossy()
        ));
        run_git_test(
            &dir,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );
        let wts = list_worktrees(&dir).unwrap();
        assert_eq!(wts.len(), 2);
        let branches: Vec<_> = wts.iter().map(|w| w.branch.as_str()).collect();
        assert!(branches.contains(&"main"));
        assert!(branches.contains(&"feature"));
    }

    #[test]
    fn compute_status_clean_repo() {
        let dir = temp_dir();
        init_repo(&dir);
        let s = compute_status(&dir).unwrap();
        assert!(s.is_clean());
    }

    #[test]
    fn compute_status_counts_staged() {
        let dir = temp_dir();
        init_repo(&dir);
        std::fs::write(dir.join("new.txt"), "x").unwrap();
        run_git_test(&dir, &["add", "new.txt"]);
        let s = compute_status(&dir).unwrap();
        assert_eq!(s.staged, 1);
        assert_eq!(s.modified, 0);
    }

    #[test]
    fn compute_status_counts_modified() {
        let dir = temp_dir();
        init_repo(&dir);
        std::fs::write(dir.join("README.md"), "changed").unwrap();
        let s = compute_status(&dir).unwrap();
        assert_eq!(s.modified, 1);
        assert_eq!(s.staged, 0);
    }

    #[test]
    fn compute_status_counts_deleted() {
        let dir = temp_dir();
        init_repo(&dir);
        std::fs::remove_file(dir.join("README.md")).unwrap();
        let s = compute_status(&dir).unwrap();
        assert_eq!(s.deleted, 1);
    }

    #[test]
    fn ahead_behind_is_zero_without_upstream() {
        let dir = temp_dir();
        init_repo(&dir);
        let s = compute_status(&dir).unwrap();
        assert_eq!(s.ahead, 0);
        assert_eq!(s.behind, 0);
    }

    #[test]
    fn create_and_remove_worktree_round_trip() {
        let dir = temp_dir();
        init_repo(&dir);
        let new_path = derive_worktree_path(&dir, "parent", "feat/x", None);
        create_worktree(&dir, "feat/x", &new_path, "main").unwrap();
        let wts = list_worktrees(&dir).unwrap();
        assert_eq!(wts.len(), 2);
        assert!(wts.iter().any(|w| w.branch == "feat/x"));

        remove_worktree(&dir, &new_path).unwrap();
        let wts = list_worktrees(&dir).unwrap();
        assert_eq!(wts.len(), 1);
    }

    #[test]
    fn derive_worktree_path_avoids_collisions() {
        let dir = temp_dir();
        let repo_root = dir.join("origin");
        std::fs::create_dir_all(&repo_root).unwrap();
        let first = derive_worktree_path(&repo_root, "repo", "branch", None);
        std::fs::create_dir_all(&first).unwrap();
        let second = derive_worktree_path(&repo_root, "repo", "branch", None);
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("-2"));
    }

    #[test]
    fn derive_worktree_path_custom_root_uses_repo_subdir() {
        let dir = temp_dir();
        let repo_root = dir.join("origin");
        let wt_root = dir.join("worktrees");
        let path = derive_worktree_path(&repo_root, "myrepo", "feat/thing", Some(&wt_root));
        // Should be <wt_root>/myrepo/feat-thing
        assert_eq!(path, wt_root.join("myrepo").join("feat-thing"));
    }

    #[test]
    fn derive_worktree_path_custom_root_avoids_collisions() {
        let dir = temp_dir();
        let repo_root = dir.join("origin");
        let wt_root = dir.join("worktrees");
        let first = derive_worktree_path(&repo_root, "repo", "branch", Some(&wt_root));
        std::fs::create_dir_all(&first).unwrap();
        let second = derive_worktree_path(&repo_root, "repo", "branch", Some(&wt_root));
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("-2"));
    }

    #[test]
    fn compute_branch_diff_only_shows_branch_unique_commits() {
        let dir = temp_dir();
        init_repo(&dir);
        // Branch off and add a commit
        run_git_test(&dir, &["checkout", "-b", "feat"]);
        std::fs::write(dir.join("feat.txt"), "feat").unwrap();
        run_git_test(&dir, &["add", "feat.txt"]);
        run_git_test(
            &dir,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "feat",
                "--quiet",
            ],
        );

        let files = compute_branch_diff(&dir, "main").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path.to_str(), Some("feat.txt"));

        // When base doesn't exist, returns empty (not an error).
        assert!(compute_branch_diff(&dir, "nonexistent").unwrap().is_empty());
    }

    #[test]
    fn compute_local_diff_lists_unstaged_and_staged_changes() {
        let dir = temp_dir();
        init_repo(&dir);
        // Modify the file to produce an unstaged edit.
        std::fs::write(dir.join("README.md"), "changed").unwrap();
        // Add a new file and stage it.
        std::fs::write(dir.join("new.txt"), "x").unwrap();
        run_git_test(&dir, &["add", "new.txt"]);

        let files = compute_local_diff(&dir).unwrap();
        let unstaged: Vec<_> = files.iter().filter(|f| !f.staged).collect();
        let staged: Vec<_> = files.iter().filter(|f| f.staged).collect();
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].path.to_str(), Some("README.md"));
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].path.to_str(), Some("new.txt"));
        assert!(staged[0].adds > 0);
    }

    #[test]
    fn ahead_behind_counts_diverged_commits() {
        // Create "remote" bare repo and "local" clone; commit on local; verify ahead=1.
        let remote = temp_dir();
        run_git_test(
            &remote,
            &["init", "--bare", "--quiet", "--initial-branch=main"],
        );

        let local = temp_dir();
        run_git_test(
            &local,
            &[
                "clone",
                "--quiet",
                remote.to_str().unwrap(),
                local.to_str().unwrap(),
            ],
        );
        // After clone, `local` has been re-init'd with origin set; it has no commits yet.
        std::fs::write(local.join("x.txt"), "x").unwrap();
        run_git_test(&local, &["add", "."]);
        run_git_test(
            &local,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "first",
                "--quiet",
            ],
        );
        run_git_test(&local, &["push", "--quiet", "-u", "origin", "main"]);
        // Now ahead=behind=0.
        let s = compute_status(&local).unwrap();
        assert_eq!(s.ahead, 0);
        assert_eq!(s.behind, 0);

        // Commit locally without pushing → ahead=1.
        std::fs::write(local.join("y.txt"), "y").unwrap();
        run_git_test(&local, &["add", "."]);
        run_git_test(
            &local,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "second",
                "--quiet",
            ],
        );
        let s = compute_status(&local).unwrap();
        assert_eq!(s.ahead, 1);
        assert_eq!(s.behind, 0);
    }

    #[test]
    fn discover_repos_finds_git_dirs_at_depth_1() {
        let dir = temp_dir();
        // layout:
        //   <dir>/a           (repo)
        //   <dir>/b           (repo)
        //   <dir>/not-a-repo  (plain dir)
        //   <dir>/.hidden     (plain dir, skipped)
        //   <dir>/c/d         (repo, below depth)
        let a = dir.join("a");
        let b = dir.join("b");
        let plain = dir.join("not-a-repo");
        let hidden = dir.join(".hidden");
        let deep = dir.join("c").join("d");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::create_dir_all(&deep).unwrap();
        init_repo(&a);
        init_repo(&b);
        init_repo(&deep);

        let found = discover_repos(&dir, 1);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn discover_repos_respects_depth() {
        let dir = temp_dir();
        let deep = dir.join("outer").join("inner");
        std::fs::create_dir_all(&deep).unwrap();
        init_repo(&deep);

        assert!(discover_repos(&dir, 1).is_empty());
        let deeper = discover_repos(&dir, 2);
        assert_eq!(deeper.len(), 1);
        assert!(deeper[0].ends_with("outer/inner"));
    }

    #[test]
    fn discover_repos_does_not_descend_into_a_repo() {
        // Repo walker should halt at `outer` and never see its inner
        // subdirectories — even if one of them also happens to look
        // like a repo root.
        let dir = temp_dir();
        let outer = dir.join("outer");
        std::fs::create_dir_all(&outer).unwrap();
        init_repo(&outer);
        // Create a sibling subdir that would be flagged as a bare-ish
        // repo root on its own (HEAD + config).  The walker should not
        // even look at it because we stop at `outer`.
        let pretend = outer.join("pretend-repo");
        std::fs::create_dir_all(&pretend).unwrap();
        std::fs::write(pretend.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(pretend.join("config"), "").unwrap();

        let found = discover_repos(&dir, 5);
        assert_eq!(found.len(), 1, "only outer repo expected: {found:?}");
        assert!(found[0].ends_with("outer"));
    }

    #[test]
    fn discover_repos_path_is_itself_a_repo_ignored() {
        // `discover_repos` is the *scan* path — handing it a repo returns
        // just that repo (the dispatch layer in main.rs covers the
        // single-repo case before calling this).
        let dir = temp_dir();
        init_repo(&dir);
        let found = discover_repos(&dir, 1);
        assert_eq!(found, vec![dir]);
    }
}
