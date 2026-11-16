//! Review → fix loop on local work, before a PR exists. Nothing is committed,
//! pushed, or posted.

use std::fmt::Write;

use anyhow::{Result, bail};

use crate::agent::Scope;
use crate::comment::{self, Changes, FixOutcome};
use crate::config::{Agent, Config};
use crate::finding::{Finding, FixReport, Status};
use crate::fix;
use crate::git;
use crate::review;

pub struct Options {
    /// Ref to compare against; `None` uses the repo's default branch.
    pub base: Option<String>,
    pub fix: bool,
    pub reviewers: Vec<Agent>,
    /// `cr loop`: review → fix rounds to run at most. `None` (`cr local`) runs one round
    /// and doesn't require its fixes to be re-reviewed.
    pub max_rounds: Option<u32>,
}

pub fn run(config: &Config, opts: Options) -> Result<()> {
    let base = match opts.base {
        Some(base) => base,
        None => git::default_base()?,
    };
    let scope = Scope::Local { base: &base };
    let out_dir = review::out_dir("local")?;
    // Drop an earlier run's raw outputs so they can't be mistaken for this run's.
    std::fs::remove_dir_all(&out_dir)?;
    std::fs::create_dir_all(&out_dir)?;
    // Findings the fixer rejected; later rounds tell the merger to drop them again.
    let mut rejected = String::new();
    let mut history = String::new();
    let mut before = git::changes_since(&base)?;

    for round in 1..=opts.max_rounds.unwrap_or(1) {
        if before.is_empty() {
            if round == 1 {
                bail!("no changes against {base} to review");
            }
            bail!("round {} fixes removed every change against {base}; check `git diff`", round - 1);
        }
        let mut raw_dir = out_dir.clone();
        if let Some(max) = opts.max_rounds {
            eprintln!("Round {round}/{max}");
            writeln!(history, "# Round {round}/{max}\n")?;
            raw_dir = review::out_dir(&format!("local/round-{round}"))?;
        }

        let (findings, used) = review::collect(config, &opts.reviewers, scope, &raw_dir, &rejected)?;
        let review_report = comment::render_findings(&findings, &used);
        println!("{review_report}");
        history.push_str(&review_report);
        review::open_report(&out_dir, &history);
        if findings.is_empty() || !opts.fix {
            return Ok(());
        }

        let (items, listing) = fix::list_findings(&findings);
        let report = fix::apply(config, scope, &items, &listing)?;
        let after = git::changes_since(&base)?;
        let changes = if after == before { Changes::None } else { Changes::Uncommitted };
        let converged = matches!(changes, Changes::None);
        let outcome = FixOutcome { items: &items, report: &report, changes };
        let fix_report = outcome.render();
        println!("{fix_report}");
        writeln!(history, "\n{fix_report}")?;
        review::open_report(&out_dir, &history);
        if !outcome.is_green() {
            bail!("not everything is green; see the summary above");
        }
        if converged {
            // Nothing changed and still green: the fixer judged every finding invalid.
            return Ok(());
        }
        rejected.push_str(&rejected_listing(&findings, &report));
        before = after;
    }
    if let Some(max) = opts.max_rounds {
        bail!("reached --max-rounds {max}; the last round's fixes were not re-reviewed");
    }
    Ok(())
}

/// One markdown line per finding the fixer marked invalid, with its location, detail and
/// the fixer's reason, so the merger can tell a repeat from a different problem.
fn rejected_listing(findings: &[Finding], report: &FixReport) -> String {
    let mut out = String::new();
    for r in report.results.iter().filter(|r| r.status == Status::Invalid) {
        if let Some(f) = findings.iter().find(|f| f.id == r.id) {
            let location = f.location().map(|l| format!(" ({l})")).unwrap_or_default();
            let (title, detail, note) = (one_line(&f.title), one_line(&f.detail), one_line(&r.note));
            writeln!(out, "- {title}{location}: {detail} Rejected: {note}").unwrap();
        }
    }
    out
}

/// Collapses whitespace so free text can't break out of its markdown list item.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::FixResult;

    #[test]
    fn lists_only_invalid_findings() {
        let finding = |id: &str, title: &str, line| Finding {
            id: id.into(),
            severity: crate::finding::Severity::Low,
            file: Some("src/a.rs".into()),
            line,
            title: title.into(),
            detail: "Some\ndetail.".into(),
            sources: vec![],
        };
        let result = |id: &str, status, note: &str| FixResult { id: id.into(), status, note: note.into() };
        let report = FixReport {
            results: vec![result("F1", Status::Fixed, "done"), result("F2", Status::Invalid, "already\n\n## guarded")],
            checks: vec![],
        };
        let listing = rejected_listing(&[finding("F1", "Off by one", None), finding("F2", "Null deref", Some(7))], &report);
        assert_eq!(listing, "- Null deref (src/a.rs:7): Some detail. Rejected: already ## guarded\n");
    }
}
