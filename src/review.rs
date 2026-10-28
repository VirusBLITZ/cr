use std::path::Path;

use anyhow::{Result, bail};

use crate::agent::{self, Mode};
use crate::comment;
use crate::config::{Agent, Config};
use crate::finding::{self, Finding, Merged};
use crate::git;
use crate::host::{Host, Pr};

pub struct Options {
    pub comment: bool,
    pub reviewers: Vec<Agent>,
}

pub fn run(config: &Config, host: &dyn Host, pr: &Pr, opts: Options) -> Result<()> {
    git::fetch(&pr.base)?;
    let out_dir = git::git_dir()?.join("cr").join(pr.number.to_string());
    std::fs::create_dir_all(&out_dir)?;

    let names: Vec<String> = opts.reviewers.iter().map(Agent::to_string).collect();
    eprintln!("Reviewing PR #{} with {}...", pr.number, names.join(", "));
    let results: Vec<Result<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = opts.reviewers.iter().map(|a| s.spawn(|| agent::review(a, pr))).collect();
        handles.into_iter().map(|h| h.join().expect("reviewer thread panicked")).collect()
    });

    let mut reviews = Vec::new();
    for (i, (name, result)) in names.iter().zip(results).enumerate() {
        match result {
            Ok(text) => {
                save(&out_dir, &format!("{i}-{}.md", opts.reviewers[i].tool), &text);
                eprintln!("  {name}: done");
                reviews.push((name.clone(), text));
            }
            Err(e) => eprintln!("  {name}: failed: {e:#}"),
        }
    }
    if reviews.is_empty() {
        bail!("all reviewers failed");
    }

    eprintln!("Merging findings with {}...", config.merger);
    let findings = merge(&config.merger, pr, &reviews)?;
    let used: Vec<String> = reviews.into_iter().map(|(name, _)| name).collect();
    println!("{}", comment::render_findings(&findings, &used));

    let body = comment::render_review(&findings, &used);
    save(&out_dir, "review.md", &body);
    if opts.comment {
        host.post_comment(pr, &body)?;
        eprintln!("Posted review comment on {}", pr.url);
    }
    Ok(())
}

fn merge(merger: &Agent, pr: &Pr, reviews: &[(String, String)]) -> Result<Vec<Finding>> {
    let mut prompt = format!(
        "Below are code reviews of PR #{} ({} -> {}) from different reviewers. \
         Merge them into one deduplicated list of findings:\n\
         - Findings describing the same problem become one; list every reviewer that reported it in `sources` \
         (use the reviewer names as given in the headings).\n\
         - Keep findings that only one reviewer reported.\n\
         - Drop anything that is not an actionable problem (praise, summaries, restated PR descriptions).\n\
         - Use the most precise file and line; `file` is relative to the repository root, and \
         `file`/`line` are null if a finding has no location.\n\
         - `detail` must be self-contained: what is wrong, why it matters, and a suggested fix.\n\
         - `severity` is one of critical, high, medium, low.\n\
         You can read the repository to check locations, but do not change anything.\n\
         Answer only with JSON matching the schema.\n",
        pr.number, pr.head, pr.base
    );
    for (name, text) in reviews {
        prompt.push_str(&format!("\n## Review from {name}\n\n{text}\n"));
    }

    let answer = agent::run(merger, &prompt, Mode::ReadOnly, Some(&finding::merged_schema()))?;
    let mut findings = finding::parse_json::<Merged>(&answer)?.findings;
    findings.sort_by_key(|f| f.severity);
    for (i, f) in findings.iter_mut().enumerate() {
        f.id = format!("F{}", i + 1);
    }
    Ok(findings)
}

/// Keeps raw outputs under `.git/cr/<pr>/` for debugging; failures here are not fatal.
fn save(dir: &Path, name: &str, text: &str) {
    if let Err(e) = std::fs::write(dir.join(name), text) {
        eprintln!("warning: could not save {name}: {e}");
    }
}
