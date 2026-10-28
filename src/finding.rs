use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Finding {
    /// Assigned by cr after merging (F1, F2, ...).
    #[serde(default)]
    pub id: String,
    pub severity: Severity,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub title: String,
    pub detail: String,
    #[serde(default)]
    pub sources: Vec<String>,
}

impl Finding {
    pub fn location(&self) -> Option<String> {
        let file = self.file.as_deref()?;
        Some(match self.line {
            Some(line) => format!("{file}:{line}"),
            None => file.to_owned(),
        })
    }
}

#[derive(Deserialize)]
pub struct Merged {
    pub findings: Vec<Finding>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Fixed,
    Invalid,
    Skipped,
}

#[derive(Deserialize, Debug)]
pub struct FixResult {
    pub id: String,
    pub status: Status,
    pub note: String,
}

#[derive(Deserialize, Debug)]
pub struct Check {
    pub command: String,
    pub passed: bool,
}

#[derive(Deserialize, Debug)]
pub struct FixReport {
    pub results: Vec<FixResult>,
    pub checks: Vec<Check>,
}

impl FixReport {
    /// Agents sometimes report every attempt of a command (fail, fix, pass);
    /// only the last run of each command counts.
    pub fn keep_final_checks(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.checks.reverse();
        self.checks.retain(|c| seen.insert(c.command.clone()));
        self.checks.reverse();
    }
}

// Schemas are written in the strict subset codex's `--output-schema` accepts:
// every property required, no additional properties.

pub fn merged_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["findings"],
        "properties": {
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["severity", "file", "line", "title", "detail", "sources"],
                    "properties": {
                        "severity": { "type": "string", "enum": ["critical", "high", "medium", "low"] },
                        "file": { "type": ["string", "null"] },
                        "line": { "type": ["integer", "null"] },
                        "title": { "type": "string" },
                        "detail": { "type": "string" },
                        "sources": { "type": "array", "items": { "type": "string" } }
                    }
                }
            }
        }
    })
}

pub fn fix_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["results", "checks"],
        "properties": {
            "results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "status", "note"],
                    "properties": {
                        "id": { "type": "string" },
                        "status": { "type": "string", "enum": ["fixed", "invalid", "skipped"] },
                        "note": { "type": "string" }
                    }
                }
            },
            "checks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["command", "passed"],
                    "properties": {
                        "command": { "type": "string" },
                        "passed": { "type": "boolean" }
                    }
                }
            }
        }
    })
}

/// Parses a model's JSON answer, tolerating code fences or prose around the object.
pub fn parse_json<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T> {
    let start = text.find('{').context("no JSON object in model output")?;
    let end = text.rfind('}').context("no JSON object in model output")?;
    serde_json::from_str(&text[start..=end]).with_context(|| format!("model returned unexpected JSON:\n{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_json() {
        let text = "Here you go:\n```json\n{\"results\": [], \"checks\": [{\"command\": \"cargo test\", \"passed\": true}]}\n```";
        let report: FixReport = parse_json(text).unwrap();
        assert_eq!(report.checks[0].command, "cargo test");
    }

    #[test]
    fn keeps_last_run_of_each_check() {
        let check = |command: &str, passed| Check { command: command.into(), passed };
        let mut report = FixReport {
            results: vec![],
            checks: vec![check("cargo test", false), check("cargo fmt", true), check("cargo test", true)],
        };
        report.keep_final_checks();
        let checks: Vec<_> = report.checks.iter().map(|c| (c.command.as_str(), c.passed)).collect();
        assert_eq!(checks, [("cargo fmt", true), ("cargo test", true)]);
    }
}
