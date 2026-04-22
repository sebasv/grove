//! GitHub PR/CI status polling via octocrab.
//!
//! Token discovery order:
//!   1. `GITHUB_TOKEN` / `GH_TOKEN` env var
//!   2. `gh auth token` (GitHub CLI — works for SSH-key users after `gh auth login`)
//!   3. None — PR badges are silently disabled

use std::path::Path;
use std::sync::Arc;

use octocrab::params::State;
use octocrab::Octocrab;

use crate::async_evt::{Event, EventSender, WorktreeId};
use crate::model::{ChecksRollup, PrState, PrStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRepo {
    pub owner: String,
    pub repo: String,
}

/// Build a shared octocrab client using the first token found in the
/// discovery chain; returns `None` if no token is available.
pub fn build_client() -> Option<Arc<Octocrab>> {
    let token = token_from_env().or_else(token_from_gh_cli)?;
    Octocrab::builder()
        .personal_token(token)
        .build()
        .ok()
        .map(Arc::new)
}

fn token_from_env() -> Option<String> {
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

/// Ask the GitHub CLI for its stored OAuth token. Works for users who
/// authenticated via `gh auth login` (including the SSH-key flow) without
/// needing a manually created PAT.
fn token_from_gh_cli() -> Option<String> {
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Discover `owner/repo` for a repository by reading the `origin` remote URL.
/// Returns `None` for non-GitHub remotes or parse failures.
pub fn discover_owner_repo(repo_root: &Path) -> Option<OwnerRepo> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_owner_repo(&url)
}

pub fn parse_owner_repo(url: &str) -> Option<OwnerRepo> {
    // Accepted shapes:
    //   git@github.com:owner/repo(.git)?
    //   https://github.com/owner/repo(.git)?
    //   ssh://git@github.com/owner/repo(.git)?
    let stripped = url
        .trim()
        .strip_prefix("git@github.com:")
        .or_else(|| url.trim().strip_prefix("https://github.com/"))
        .or_else(|| url.trim().strip_prefix("ssh://git@github.com/"))?;
    let stripped = stripped.trim_end_matches('/');
    let stripped = stripped.strip_suffix(".git").unwrap_or(stripped);
    let mut parts = stripped.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(OwnerRepo { owner, repo })
}

/// Fire off an async fetch for this worktree's PR status.
pub fn spawn_pr_fetch(
    client: Arc<Octocrab>,
    owner_repo: OwnerRepo,
    branch: String,
    id: WorktreeId,
    tx: EventSender,
) {
    tokio::spawn(async move {
        if let Some(status) =
            fetch_pr_status(&client, &owner_repo.owner, &owner_repo.repo, &branch).await
        {
            let _ = tx.send(Event::PrStatusReady(id, status));
        }
    });
}

async fn fetch_pr_status(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Option<PrStatus> {
    let head_label = format!("{owner}:{branch}");
    let prs = client
        .pulls(owner, repo)
        .list()
        .state(State::All)
        .head(head_label)
        .per_page(1)
        .send()
        .await
        .ok()?;
    let pr = prs.into_iter().next()?;

    let state = if pr.merged_at.is_some() {
        PrState::Merged
    } else if matches!(pr.state, Some(octocrab::models::IssueState::Closed)) {
        PrState::Closed
    } else if pr.draft.unwrap_or(false) {
        PrState::Draft
    } else {
        PrState::Open
    };

    let head_sha = pr.head.sha.clone();
    let checks = if state == PrState::Open || state == PrState::Draft {
        fetch_checks_rollup(client, owner, repo, &head_sha).await
    } else {
        ChecksRollup::None
    };

    Some(PrStatus {
        number: pr.number as u32,
        state,
        checks,
    })
}

async fn fetch_checks_rollup(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    sha: &str,
) -> ChecksRollup {
    let Ok(body): Result<serde_json::Value, _> = client
        .get::<serde_json::Value, _, ()>(
            format!("/repos/{owner}/{repo}/commits/{sha}/check-runs"),
            None,
        )
        .await
    else {
        return ChecksRollup::None;
    };

    let Some(runs) = body.get("check_runs").and_then(|v| v.as_array()) else {
        return ChecksRollup::None;
    };
    if runs.is_empty() {
        return ChecksRollup::None;
    }

    let mut pending = false;
    let mut failing = false;
    for run in runs {
        let status = run.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let conclusion = run
            .get("conclusion")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if status != "completed" {
            pending = true;
        } else {
            match conclusion {
                "success" | "neutral" | "skipped" => {}
                "" => pending = true,
                _ => failing = true,
            }
        }
    }

    if failing {
        ChecksRollup::Failing
    } else if pending {
        ChecksRollup::Pending
    } else {
        ChecksRollup::Passing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_shortcut() {
        let got = parse_owner_repo("git@github.com:sebasv/grove.git").unwrap();
        assert_eq!(got.owner, "sebasv");
        assert_eq!(got.repo, "grove");
    }

    #[test]
    fn parses_https_with_git_suffix() {
        let got = parse_owner_repo("https://github.com/sebasv/grove.git").unwrap();
        assert_eq!(got.owner, "sebasv");
        assert_eq!(got.repo, "grove");
    }

    #[test]
    fn parses_https_without_git_suffix() {
        let got = parse_owner_repo("https://github.com/sebasv/grove").unwrap();
        assert_eq!(got.owner, "sebasv");
        assert_eq!(got.repo, "grove");
    }

    #[test]
    fn parses_ssh_scheme() {
        let got = parse_owner_repo("ssh://git@github.com/sebasv/grove.git").unwrap();
        assert_eq!(got.owner, "sebasv");
        assert_eq!(got.repo, "grove");
    }

    #[test]
    fn rejects_non_github_remote() {
        assert!(parse_owner_repo("git@gitlab.com:a/b.git").is_none());
        assert!(parse_owner_repo("file:///tmp/repo").is_none());
    }
}
