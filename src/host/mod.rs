//! Git hosting abstraction. To support a new host (GitLab, Gitea, ...), add a
//! module implementing [`Host`] and register it in [`all`].

mod github;

use anyhow::{Result, bail};

use crate::config::Config;
use crate::git;

#[derive(Debug, Clone, PartialEq)]
pub struct Pr {
    pub number: u64,
    pub url: String,
    pub head: String,
    pub base: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub author: String,
    pub body: String,
    /// Set for inline review comments.
    pub path: Option<String>,
    pub line: Option<u32>,
}

pub trait Host {
    fn name(&self) -> &'static str;
    /// Whether `remote_url` (e.g. the origin URL) belongs to this host.
    fn matches_remote(&self, remote_url: &str) -> bool;
    /// Parses a PR URL into (repo slug, PR number).
    fn parse_url(&self, url: &str) -> Option<(String, u64)>;
    /// Slug of the repository in the current directory, comparable to `parse_url`'s.
    fn repo_slug(&self) -> Result<String>;
    /// Looks up a PR by number or branch name; an empty selector means the current branch.
    fn find_pr(&self, selector: &str) -> Result<Pr>;
    /// Checks out the PR's head branch (including PRs from forks).
    fn checkout(&self, pr: &Pr) -> Result<()>;
    /// All conversation, review, and inline comments, oldest first per kind.
    fn comments(&self, pr: &Pr) -> Result<Vec<Comment>>;
    fn post_comment(&self, pr: &Pr, body: &str) -> Result<()>;
}

fn all() -> Vec<Box<dyn Host>> {
    vec![Box::new(github::GitHub)]
}

pub fn detect(config: &Config) -> Result<Box<dyn Host>> {
    let mut hosts = all();
    let names = hosts.iter().map(|h| h.name()).collect::<Vec<_>>().join(", ");
    if let Some(name) = &config.host {
        return match hosts.iter().position(|h| h.name() == name) {
            Some(i) => Ok(hosts.swap_remove(i)),
            None => bail!("unknown host `{name}` in config (supported: {names})"),
        };
    }
    let remote = git::origin_url()?;
    match hosts.iter().position(|h| h.matches_remote(&remote)) {
        Some(i) => Ok(hosts.swap_remove(i)),
        None => bail!(
            "unsupported git host for origin `{remote}` (supported: {names}); set `host` in .cr.toml to override"
        ),
    }
}
