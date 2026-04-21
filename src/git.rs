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

pub fn compute_branch_diff(
    worktree_path: &Path,
    base_branch: &str,
) -> Result<Vec<DiffFile>> {
    let repo = Repository::open(worktree_path)
        .with_context(|| format!("opening {}", worktree_path.display()))?;
    let head = repo.head().context("resolving HEAD")?;
    let head_oid = head.target().ok_or_else(|| anyhow::anyhow!("HEAD has no OID"))?;

    let base_ref = match repo.find_branch(base_branch, BranchType::Local) {
        Ok(b) => b,
        Err(_) => return Ok(Vec::new()),
    };
    let base_oid = base_ref
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("base branch has no OID"))?;

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

    let head_tree = match repo.head().and_then(|h| h.peel_to_tree()) {
        Ok(t) => Some(t),
        Err(_) => None,
    };
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
                let header =
                    std::str::from_utf8(hunk.header()).unwrap_or("").to_string();
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
                    let content =
                        std::str::from_utf8(line.content()).unwrap_or("").to_string();
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

pub fn create_worktree(
    repo_root: &Path,
    branch: &str,
    path: &Path,
    base: &str,
) -> Result<()> {
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

pub fn derive_worktree_path(repo_root: &Path, repo_name: &str, branch: &str) -> PathBuf {
    let parent = repo_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let sanitised = branch.replace('/', "-");
    let mut candidate = parent.join(format!("{repo_name}-{sanitised}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{repo_name}-{sanitised}-{counter}"));
        counter += 1;
    }
    candidate
}

pub fn stage_path(worktree_path: &Path, file_path: &Path) -> Result<()> {
    run_git_cmd(worktree_path, &["add", "--", &file_path.display().to_string()])
}

pub fn unstage_path(worktree_path: &Path, file_path: &Path) -> Result<()> {
    run_git_cmd(
        worktree_path,
        &["restore", "--staged", "--", &file_path.display().to_string()],
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

pub fn list_worktrees(repo_root: &Path) -> Result<Vec<Worktree>> {
    let repo = Repository::open(repo_root)
        .with_context(|| format!("opening repository at {}", repo_root.display()))?;
    let mut out = Vec::new();

    // Primary checkout.
    if let Some(workdir) = repo.workdir() {
        if let Some(branch) = branch_name_of_head(&repo) {
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
        let new_path = derive_worktree_path(&dir, "parent", "feat/x");
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
        // Isolate under a fresh subdir so we don't collide with anything in
        // $TMPDIR left over from other runs.
        let dir = temp_dir();
        let repo_root = dir.join("origin");
        std::fs::create_dir_all(&repo_root).unwrap();
        let first = derive_worktree_path(&repo_root, "repo", "branch");
        std::fs::create_dir_all(&first).unwrap();
        let second = derive_worktree_path(&repo_root, "repo", "branch");
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
        run_git_test(&remote, &["init", "--bare", "--quiet", "--initial-branch=main"]);

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
}
