use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, bail};

use crate::cmd;

fn git(args: &[&str]) -> Result<String> {
    Ok(cmd::run(Command::new("git").args(args), None)?.trim().to_owned())
}

pub fn repo_root() -> Result<PathBuf> {
    git(&["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|_| anyhow::anyhow!("not inside a git repository"))
}

pub fn git_dir() -> Result<PathBuf> {
    git(&["rev-parse", "--absolute-git-dir"]).map(PathBuf::from)
}

pub fn origin_url() -> Result<String> {
    git(&["remote", "get-url", "origin"])
}

pub fn has_changes() -> Result<bool> {
    Ok(!git(&["status", "--porcelain"])?.is_empty())
}

pub fn ensure_clean() -> Result<()> {
    if has_changes()? {
        bail!("working tree has uncommitted changes; commit or stash them first");
    }
    Ok(())
}

pub fn pull() -> Result<()> {
    git(&["pull", "--ff-only"]).map(drop)
}

pub fn fetch(branch: &str) -> Result<()> {
    git(&["fetch", "origin", branch]).map(drop)
}

/// Stages everything, commits, pushes, and returns the short SHA.
pub fn commit_and_push(message: &str) -> Result<String> {
    git(&["add", "-A"])?;
    git(&["commit", "-m", message])?;
    git(&["push"])?;
    git(&["rev-parse", "--short", "HEAD"])
}

/// The branch local changes are compared against: origin's default branch,
/// else a local `main` or `master`.
pub fn default_base() -> Result<String> {
    if let Ok(base) = git(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        return Ok(base);
    }
    for branch in ["main", "master"] {
        if git(&["rev-parse", "--verify", "--quiet", branch]).is_ok() {
            return Ok(branch.to_owned());
        }
    }
    bail!("could not determine the base branch; pass --base")
}

/// Fingerprint of everything that differs from `base`: the diff since the
/// merge base plus untracked files and their contents. Equal fingerprints
/// mean nothing changed.
pub fn changes_since(base: &str) -> Result<String> {
    let merge_base = git(&["merge-base", base, "HEAD"])?;
    let diff = git(&["diff", &merge_base])?;
    let untracked = git(&["ls-files", "--others", "--exclude-standard"])?;
    let mut fingerprint = format!("{diff}\n{untracked}");
    if !untracked.is_empty() {
        let mut args = vec!["hash-object", "--"];
        args.extend(untracked.lines());
        fingerprint.push_str(&git(&args)?);
    }
    Ok(fingerprint.trim().to_owned())
}
