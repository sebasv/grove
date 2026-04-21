use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::model::Worktree;

pub fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn list_worktrees(repo_root: &Path) -> Result<Vec<Worktree>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .with_context(|| format!("running `git worktree list` in {}", repo_root.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "`git worktree list` failed in {}: {}",
            repo_root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(parse_porcelain(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_porcelain(input: &str) -> Vec<Worktree> {
    // Porcelain format: one block per worktree, blocks separated by blank lines.
    // Each block has `worktree <path>` on the first line, then HEAD sha, then
    // either `branch refs/heads/<name>` or `detached` (or `bare`).
    let mut worktrees = Vec::new();
    let mut is_primary = true;
    for block in input.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        let mut skip = false;
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(rest));
            } else if let Some(rest) = line.strip_prefix("branch ") {
                branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
            } else if line == "detached" || line == "bare" {
                skip = true;
            }
        }
        if let (false, Some(path), Some(branch)) = (skip, path, branch) {
            worktrees.push(Worktree {
                branch,
                path,
                is_primary,
            });
        }
        is_primary = false;
    }
    worktrees
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_with_primary_and_linked() {
        let input = "\
worktree /Users/sebas/dev/grove
HEAD 0123456789abcdef0123456789abcdef01234567
branch refs/heads/main

worktree /Users/sebas/dev/grove-feat-sidebar
HEAD fedcba9876543210fedcba9876543210fedcba98
branch refs/heads/feat/sidebar
";
        let got = parse_porcelain(input);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].branch, "main");
        assert!(got[0].is_primary);
        assert_eq!(got[1].branch, "feat/sidebar");
        assert!(!got[1].is_primary);
    }

    #[test]
    fn skips_detached_and_bare_worktrees() {
        let input = "\
worktree /r
HEAD 0123
branch refs/heads/main

worktree /r-detached
HEAD 4567
detached

worktree /r-bare
bare
";
        let got = parse_porcelain(input);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].branch, "main");
    }

    #[test]
    fn handles_empty_input() {
        assert!(parse_porcelain("").is_empty());
        assert!(parse_porcelain("\n\n").is_empty());
    }
}
