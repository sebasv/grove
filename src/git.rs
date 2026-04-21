use std::path::Path;

use anyhow::{Context, Result};
use git2::{BranchType, Repository, Status, StatusOptions};

use crate::model::{Worktree, WorktreeStatus};

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

    fn run_git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        run_git(dir, &["add", "."]);
        run_git(
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
        run_git(
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
        run_git(&dir, &["add", "new.txt"]);
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
    fn ahead_behind_counts_diverged_commits() {
        // Create "remote" bare repo and "local" clone; commit on local; verify ahead=1.
        let remote = temp_dir();
        run_git(&remote, &["init", "--bare", "--quiet", "--initial-branch=main"]);

        let local = temp_dir();
        run_git(
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
        run_git(&local, &["add", "."]);
        run_git(
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
        run_git(&local, &["push", "--quiet", "-u", "origin", "main"]);
        // Now ahead=behind=0.
        let s = compute_status(&local).unwrap();
        assert_eq!(s.ahead, 0);
        assert_eq!(s.behind, 0);

        // Commit locally without pushing → ahead=1.
        std::fs::write(local.join("y.txt"), "y").unwrap();
        run_git(&local, &["add", "."]);
        run_git(
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
