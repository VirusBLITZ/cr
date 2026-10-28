use std::fmt::Write;

use anyhow::{Result, bail};

use crate::agent::{self, Mode};
use crate::comment::{self, FIX_MARKER, FixOutcome, Item, REVIEW_MARKER};
use crate::config::Config;
use crate::finding::{self, FixReport};
use crate::git;
use crate::host::{Host, Pr};
use crate::status;

pub struct Options {
    pub push: bool,
    pub comment: bool,
}

pub fn run(config: &Config, host: &dyn Host, pr: &Pr, opts: Options) -> Result<()> {
    let comments = host.comments(pr)?;

    // Findings from the latest cr review, plus every other human/bot comment.
    let findings = comments.iter().rev().find_map(|c| comment::parse_review(&c.body)).unwrap_or_default();
    let others: Vec<_> = comments
        .iter()
        .filter(|c| !c.body.trim().is_empty())
        .filter(|c| !c.body.starts_with(REVIEW_MARKER) && !c.body.starts_with(FIX_MARKER))
        .collect();
    if findings.is_empty() && others.is_empty() {
        bail!("PR #{} has no review findings or comments to fix", pr.number);
    }

    let mut items = Vec::new();
    let mut listing = String::new();
    for f in &findings {
        items.push(Item { id: f.id.clone(), title: f.title.clone() });
        let location = f.location().map(|l| format!(" {l}")).unwrap_or_default();
        let severity = serde_json::to_value(f.severity)?;
        let severity = severity.as_str().unwrap_or_default();
        writeln!(listing, "### {} [{severity}]{location}: {}\n\n{}\n", f.id, f.title, f.detail)?;
    }
    for (i, c) in others.iter().enumerate() {
        let id = format!("C{}", i + 1);
        let first_line = c.body.lines().next().unwrap_or_default();
        items.push(Item { id: id.clone(), title: format!("@{}: {}", c.author, truncate(first_line, 80)) });
        let location = match (&c.path, c.line) {
            (Some(path), Some(line)) => format!(" on {path}:{line}"),
            (Some(path), None) => format!(" on {path}"),
            _ => String::new(),
        };
        writeln!(listing, "### {id}: comment by @{}{location}\n\n{}\n", c.author, c.body)?;
    }

    let prompt = format!(
        "You are addressing review feedback on PR #{} ({} -> {}). The PR branch is checked out in the \
         current directory.\n\n\
         For every item below (IDs F* are review findings, C* are PR comments):\n\
         1. Validate it against the current code. If it is not a real problem, is already fixed, or is not \
         actionable (e.g. \"LGTM\", CI bot noise), mark it `invalid` and say why in `note`.\n\
         2. If it is valid, fix it with a minimal, focused change and mark it `fixed`.\n\
         3. If you cannot fix it safely, mark it `skipped` and explain in `note`.\n\n\
         Then run the project's build, tests and linters (whatever this repository uses) and report every \
         command in `checks`, once, with its final result. If a check fails because of your changes, fix it and rerun. \
         Do not commit or push.\n\
         Answer only with JSON matching the schema, with one `results` entry per item.\n\n{listing}",
        pr.number, pr.head, pr.base
    );
    let answer = status::run_one(format!("fixing {} item(s) with {}", items.len(), config.fixer), || {
        agent::run(&config.fixer, &prompt, Mode::Write, Some(&finding::fix_schema()))
    })?;
    let mut report: FixReport = finding::parse_json(&answer)?;
    report.keep_final_checks();

    let changed = git::has_changes()?;
    let commit = if changed && opts.push {
        Some(git::commit_and_push("fix: address review findings")?)
    } else {
        None
    };

    let outcome = FixOutcome { items: &items, report: &report, commit: commit.as_deref() };
    let body = outcome.render();
    println!("{body}");

    if changed && !opts.push {
        eprintln!("Changes left uncommitted (--no-push); not posting a comment.");
    } else if opts.comment {
        host.post_comment(pr, &body)?;
        eprintln!("Posted fix comment on {}", pr.url);
    }
    if !outcome.is_green() {
        bail!("not everything is green; see the summary above");
    }
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}
