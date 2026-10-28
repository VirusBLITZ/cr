//! Markdown for the PR comments cr posts, and parsing its own review comment back.

use std::fmt::Write;

use crate::finding::{Finding, FixReport, Severity, Status};

pub const REVIEW_MARKER: &str = "<!-- cr:review -->";
pub const FIX_MARKER: &str = "<!-- cr:fix -->";
const DATA_FENCE: &str = "```json cr-data\n";

/// Human-readable findings list, also printed to the terminal.
pub fn render_findings(findings: &[Finding], reviewers: &[String]) -> String {
    let mut out = format!("## cr review: {} finding(s)\n\n", findings.len());
    writeln!(out, "_Reviewed by {}_\n", reviewers.join(", ")).unwrap();
    if findings.is_empty() {
        out.push_str("No findings. 🎉\n");
    }
    for severity in [Severity::Critical, Severity::High, Severity::Medium, Severity::Low] {
        let group: Vec<_> = findings.iter().filter(|f| f.severity == severity).collect();
        if group.is_empty() {
            continue;
        }
        let heading = match severity {
            Severity::Critical => "🔴 Critical",
            Severity::High => "🟠 High",
            Severity::Medium => "🟡 Medium",
            Severity::Low => "🔵 Low",
        };
        writeln!(out, "### {heading}\n").unwrap();
        for f in group {
            write!(out, "- **{}** ", f.id).unwrap();
            if let Some(location) = f.location() {
                write!(out, "`{location}` ").unwrap();
            }
            write!(out, "{}", f.title).unwrap();
            if !f.sources.is_empty() {
                write!(out, " _({})_", f.sources.join(", ")).unwrap();
            }
            out.push('\n');
            for line in f.detail.lines() {
                writeln!(out, "  {line}").unwrap();
            }
        }
        out.push('\n');
    }
    out
}

/// Full review comment: marker, findings, and the machine-readable data `cr fix` reads.
pub fn render_review(findings: &[Finding], reviewers: &[String]) -> String {
    let data = serde_json::to_string_pretty(findings).expect("findings serialize");
    format!(
        "{REVIEW_MARKER}\n{}\n<details><summary>cr data</summary>\n\n{DATA_FENCE}{data}\n```\n</details>\n",
        render_findings(findings, reviewers)
    )
}

pub fn parse_review(body: &str) -> Option<Vec<Finding>> {
    if !body.starts_with(REVIEW_MARKER) {
        return None;
    }
    let start = body.find(DATA_FENCE)? + DATA_FENCE.len();
    let end = start + body[start..].find("\n```")?;
    serde_json::from_str(&body[start..end]).ok()
}

/// An item the fixer was asked to handle: a cr finding or another PR comment.
pub struct Item {
    pub id: String,
    pub title: String,
}

/// What happened to the fixer's code changes.
pub enum Changes<'a> {
    None,
    Pushed(&'a str),
    Uncommitted,
}

pub struct FixOutcome<'a> {
    pub items: &'a [Item],
    pub report: &'a FixReport,
    pub changes: Changes<'a>,
}

impl FixOutcome<'_> {
    pub fn is_green(&self) -> bool {
        let all_handled = self.items.iter().all(|item| {
            self.report.results.iter().any(|r| r.id == item.id && r.status != Status::Skipped)
        });
        let checks_pass = !self.report.checks.is_empty() && self.report.checks.iter().all(|c| c.passed);
        all_handled && checks_pass
    }

    /// The PR comment: the summary plus the marker cr uses to recognise it.
    pub fn comment(&self) -> String {
        format!("{FIX_MARKER}\n{}", self.render())
    }

    pub fn render(&self) -> String {
        let mut out = if self.is_green() {
            "## cr fix: all findings addressed, checks green ✅\n\n".to_owned()
        } else {
            "## cr fix: not everything is green ⚠️\n\n".to_owned()
        };
        match self.changes {
            Changes::None => out.push_str("No code changes were needed.\n\n"),
            Changes::Pushed(sha) => writeln!(out, "Fixes pushed in {sha}.\n").unwrap(),
            Changes::Uncommitted => out.push_str("Fixes are left uncommitted in the working tree.\n\n"),
        }

        out.push_str("| ID | Status | Finding | Note |\n|---|---|---|---|\n");
        for item in self.items {
            let result = self.report.results.iter().find(|r| r.id == item.id);
            let (status, note) = match result {
                Some(r) => (
                    match r.status {
                        Status::Fixed => "✅ fixed",
                        Status::Invalid => "➖ not an issue",
                        Status::Skipped => "⚠️ skipped",
                    },
                    r.note.as_str(),
                ),
                None => ("⚠️ not reported", ""),
            };
            writeln!(out, "| {} | {status} | {} | {} |", item.id, cell(&item.title), cell(note)).unwrap();
        }

        out.push_str("\n### Checks\n\n");
        if self.report.checks.is_empty() {
            out.push_str("⚠️ No checks were run.\n");
        }
        for check in &self.report.checks {
            let mark = if check.passed { "✅" } else { "❌" };
            writeln!(out, "- {mark} `{}`", check.command).unwrap();
        }
        out
    }
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Check, FixResult};

    fn finding() -> Finding {
        Finding {
            id: "F1".into(),
            severity: Severity::High,
            file: Some("src/lib.rs".into()),
            line: Some(3),
            title: "Off by one".into(),
            detail: "Loop skips the last element.\n```rust\nfor i in 0..n-1\n```".into(),
            sources: vec!["codex".into(), "claude".into()],
        }
    }

    #[test]
    fn review_comment_roundtrips() {
        let findings = vec![finding()];
        let body = render_review(&findings, &["codex".into()]);
        assert!(body.contains("`src/lib.rs:3` Off by one"));
        assert_eq!(parse_review(&body), Some(findings));
        assert_eq!(parse_review("just a comment"), None);
    }

    #[test]
    fn green_requires_all_handled_and_checks_passing() {
        let items = [Item { id: "F1".into(), title: "x".into() }, Item { id: "C1".into(), title: "y".into() }];
        let mut report = FixReport {
            results: vec![
                FixResult { id: "F1".into(), status: Status::Fixed, note: String::new() },
                FixResult { id: "C1".into(), status: Status::Invalid, note: String::new() },
            ],
            checks: vec![Check { command: "cargo test".into(), passed: true }],
        };
        assert!(FixOutcome { items: &items, report: &report, changes: Changes::None }.is_green());

        report.checks[0].passed = false;
        assert!(!FixOutcome { items: &items, report: &report, changes: Changes::None }.is_green());

        report.checks[0].passed = true;
        report.results.pop();
        let outcome = FixOutcome { items: &items, report: &report, changes: Changes::None };
        assert!(!outcome.is_green());
        assert!(outcome.render().contains("| C1 | ⚠️ not reported |"));
    }
}
