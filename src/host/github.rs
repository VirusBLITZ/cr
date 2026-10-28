use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Comment, Host, Pr};
use crate::cmd;

pub struct GitHub;

fn gh(args: &[&str], stdin: Option<&str>) -> Result<String> {
    cmd::run(Command::new("gh").args(args), stdin)
}

fn gh_json<T: for<'de> Deserialize<'de>>(args: &[&str]) -> Result<T> {
    let out = gh(args, None)?;
    serde_json::from_str(&out).with_context(|| format!("unexpected output from `gh {}`", args.join(" ")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrView {
    number: u64,
    url: String,
    head_ref_name: String,
    base_ref_name: String,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

#[derive(Deserialize)]
struct Body {
    author: Option<Author>,
    body: String,
}

#[derive(Deserialize)]
struct Conversation {
    comments: Vec<Body>,
    reviews: Vec<Body>,
}

#[derive(Deserialize)]
struct InlineComment {
    author: Option<String>,
    body: String,
    path: Option<String>,
    line: Option<u32>,
}

impl Host for GitHub {
    fn name(&self) -> &'static str {
        "github"
    }

    fn matches_remote(&self, remote_url: &str) -> bool {
        remote_url.contains("github.com")
    }

    fn parse_url(&self, url: &str) -> Option<(String, u64)> {
        let path = url.split_once("github.com/")?.1;
        let mut parts = path.split('/');
        let (owner, repo, kind, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if kind != "pull" {
            return None;
        }
        let number = number.split(['?', '#']).next()?.parse().ok()?;
        Some((format!("{owner}/{repo}"), number))
    }

    fn repo_slug(&self) -> Result<String> {
        Ok(gh(&["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"], None)?.trim().to_owned())
    }

    fn find_pr(&self, selector: &str) -> Result<Pr> {
        let mut args = vec!["pr", "view"];
        if !selector.is_empty() {
            args.push(selector);
        }
        args.extend(["--json", "number,url,headRefName,baseRefName"]);
        let pr: PrView = gh_json(&args)?;
        Ok(Pr { number: pr.number, url: pr.url, head: pr.head_ref_name, base: pr.base_ref_name })
    }

    fn checkout(&self, pr: &Pr) -> Result<()> {
        gh(&["pr", "checkout", &pr.number.to_string()], None).map(drop)
    }

    fn comments(&self, pr: &Pr) -> Result<Vec<Comment>> {
        let number = pr.number.to_string();
        let conversation: Conversation = gh_json(&["pr", "view", &number, "--json", "comments,reviews"])?;
        let mut comments: Vec<Comment> = conversation
            .comments
            .into_iter()
            .chain(conversation.reviews)
            .map(|c| Comment {
                author: c.author.map(|a| a.login).unwrap_or_default(),
                body: c.body,
                path: None,
                line: None,
            })
            .collect();

        // `--jq` emits one JSON object per line, which sidesteps how `--paginate` joins pages.
        let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments");
        let inline = gh(
            &["api", &endpoint, "--paginate", "--jq", ".[] | {author: .user.login, body, path, line}"],
            None,
        )?;
        for line in inline.lines().filter(|l| !l.trim().is_empty()) {
            let c: InlineComment = serde_json::from_str(line).context("unexpected inline comment from `gh api`")?;
            comments.push(Comment { author: c.author.unwrap_or_default(), body: c.body, path: c.path, line: c.line });
        }
        Ok(comments)
    }

    fn post_comment(&self, pr: &Pr, body: &str) -> Result<()> {
        gh(&["pr", "comment", &pr.number.to_string(), "--body-file", "-"], Some(body)).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pr_urls() {
        let gh = GitHub;
        assert_eq!(gh.parse_url("https://github.com/o/r/pull/42"), Some(("o/r".into(), 42)));
        assert_eq!(gh.parse_url("https://github.com/o/r/pull/42/files#diff"), Some(("o/r".into(), 42)));
        assert_eq!(gh.parse_url("https://github.com/o/r/pull/7?w=1"), Some(("o/r".into(), 7)));
        assert_eq!(gh.parse_url("https://github.com/o/r/issues/42"), None);
        assert_eq!(gh.parse_url("https://gitlab.com/o/r/-/merge_requests/1"), None);
    }
}
