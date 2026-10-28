use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::agent::{self, Mode, Scope};
use crate::comment;
use crate::config::{Agent, Config};
use crate::finding::{self, Finding, Merged};
use crate::git;
use crate::host::{Host, Pr};
use crate::status;

pub struct Options {
    pub comment: bool,
    pub reviewers: Vec<Agent>,
}

pub fn run(config: &Config, host: &dyn Host, pr: &Pr, opts: Options) -> Result<()> {
    git::fetch(&pr.base)?;
    let out_dir = out_dir(&pr.number.to_string())?;
    let (findings, used) = collect(config, &opts.reviewers, Scope::Pr(pr), &out_dir)?;
    println!("{}", comment::render_findings(&findings, &used));

    let body = comment::render_review(&findings, &used);
    save(&out_dir, "review.md", &body);
    if opts.comment {
        host.post_comment(pr, &body)?;
        eprintln!("Posted review comment on {}", pr.url);
    }
    Ok(())
}

/// `.git/cr/<name>/`, where raw outputs are kept for debugging.
pub fn out_dir(name: &str) -> Result<PathBuf> {
    let dir = git::git_dir()?.join("cr").join(name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Runs every reviewer on `scope` in parallel and merges their findings.
/// Returns the findings and the names of the reviewers that succeeded.
pub fn collect(
    config: &Config,
    reviewers: &[Agent],
    scope: Scope,
    out_dir: &Path,
) -> Result<(Vec<Finding>, Vec<String>)> {
    let (reviewers, missing): (Vec<&Agent>, Vec<&Agent>) =
        reviewers.iter().partition(|a| agent::skill_installed(a));
    for a in missing {
        eprintln!("Skipping {a}: skill not installed");
    }
    if reviewers.is_empty() {
        bail!("no reviewers to run");
    }

    eprintln!("Reviewing {}:", scope.title());
    let names: Vec<String> = reviewers.iter().map(|a| a.to_string()).collect();
    let jobs = reviewers.iter().map(|&a| move || agent::review(a, scope)).collect();
    let results = status::run(&names, jobs);

    let mut reviews = Vec::new();
    for (i, (name, result)) in names.iter().zip(results).enumerate() {
        match result {
            Ok(text) => {
                save(out_dir, &format!("{i}-{}.md", reviewers[i].tool), &text);
                reviews.push((name.clone(), text));
            }
            Err(e) => eprintln!("{name} failed: {e:#}"),
        }
    }
    if reviews.is_empty() {
        bail!("all reviewers failed");
    }

    let findings = status::run_one(format!("merging with {}", config.merger), || {
        merge(&config.merger, scope, &reviews)
    })?;
    let used = reviews.into_iter().map(|(name, _)| name).collect();
    Ok((findings, used))
}

fn merge(merger: &Agent, scope: Scope, reviews: &[(String, String)]) -> Result<Vec<Finding>> {
    let mut prompt = format!(
        "Below are code reviews of {} from different reviewers. \
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
        scope.describe()
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

/// Saves a raw output; failures here are not fatal.
pub fn save(dir: &Path, name: &str, text: &str) {
    if let Err(e) = std::fs::write(dir.join(name), text) {
        eprintln!("warning: could not save {name}: {e}");
    }
}
