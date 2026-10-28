//! Drives the AI CLIs. Supporting another CLI means adding a `Tool` variant
//! and a match arm in each function here.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::cmd;
use crate::config::{Agent, Tool};
use crate::host::Pr;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    ReadOnly,
    /// May edit files and run commands (tests, linters) in the repo.
    Write,
}

/// What a review or fix is about.
#[derive(Clone, Copy)]
pub enum Scope<'a> {
    Pr(&'a Pr),
    /// Local work, committed and uncommitted, against a base ref. No PR needed.
    Local { base: &'a str },
}

impl Scope<'_> {
    /// Git ref the changes are compared against.
    pub fn base_ref(&self) -> String {
        match self {
            Scope::Pr(pr) => format!("origin/{}", pr.base),
            Scope::Local { base } => base.to_string(),
        }
    }

    pub fn title(&self) -> String {
        match self {
            Scope::Pr(pr) => format!("PR #{}", pr.number),
            Scope::Local { base } => format!("local changes against {base}"),
        }
    }

    /// Sentence fragment telling an agent exactly which changes to look at.
    pub fn describe(&self) -> String {
        match self {
            Scope::Pr(pr) => format!(
                "PR #{}: the changes on branch `{}` against `origin/{}` (`git diff origin/{}...HEAD`)",
                pr.number, pr.head, pr.base, pr.base
            ),
            Scope::Local { base } => format!(
                "the local changes against `{base}`, both committed and uncommitted \
                 (`git diff $(git merge-base {base} HEAD)` plus untracked files from `git status`)"
            ),
        }
    }
}

const CLAUDE_READ_TOOLS: &str = "Read,Grep,Glob,Bash(git:*),Bash(gh:*)";
const CLAUDE_WRITE_TOOLS: &str = "Read,Grep,Glob,Edit,Write,Bash";

/// Runs the agent's review skill, or the tool's built-in review, on `scope`.
pub fn review(agent: &Agent, scope: Scope) -> Result<String> {
    match (agent.tool, &agent.skill) {
        (Tool::Codex, Some(skill)) => {
            let prompt = format!("${skill} Review {}. Do not change any files.", scope.describe());
            run(agent, &prompt, Mode::ReadOnly, None)
        }
        (Tool::Claude, skill) => {
            let skill = skill.as_deref().unwrap_or("code-review high");
            let prompt = match scope {
                Scope::Pr(pr) => format!("/{skill} {}", pr.number),
                Scope::Local { .. } => format!("/{skill} Review {}.", scope.describe()),
            };
            // The prompt goes through stdin: `--allowedTools` is variadic and would swallow it.
            let mut cmd = claude(agent);
            cmd.args(["--allowedTools", CLAUDE_READ_TOOLS]).args(&agent.args);
            claude_result(&cmd::run(&mut cmd, Some(&prompt))?)
        }
        (Tool::Codex, None) => {
            let out_file = TempFile::new("review.txt");
            let mut cmd = codex(agent);
            cmd.args(["exec", "review", "--base", &scope.base_ref(), "--output-last-message"])
                .arg(&out_file.0)
                .args(&agent.args);
            cmd::run(&mut cmd, None)?;
            std::fs::read_to_string(&out_file.0).context("codex wrote no review")
        }
    }
}

/// Whether the agent's skill (if any) can be found. Claude skills also come
/// from plugins and built-ins, so they are assumed to be present.
pub fn skill_installed(agent: &Agent) -> bool {
    let Some(skill) = &agent.skill else { return true };
    match agent.tool {
        Tool::Claude => true,
        Tool::Codex => {
            let codex_home = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".codex")));
            let agents_home = dirs::home_dir().map(|h| h.join(".agents"));
            // The repo root is the working directory, so repo-local skills are relative.
            [codex_home, agents_home, Some(".codex".into()), Some(".agents".into())]
                .into_iter()
                .flatten()
                .any(|dir| dir.join("skills").join(skill).join("SKILL.md").is_file())
        }
    }
}

/// Runs a free-form prompt. With a `schema`, the answer is JSON matching it.
pub fn run(agent: &Agent, prompt: &str, mode: Mode, schema: Option<&Value>) -> Result<String> {
    match agent.tool {
        Tool::Codex => {
            let out_file = TempFile::new("out.txt");
            let schema_file = TempFile::new("schema.json");
            let mut cmd = codex(agent);
            let sandbox = if mode == Mode::Write { "workspace-write" } else { "read-only" };
            cmd.args(["exec", "--sandbox", sandbox, "--output-last-message"]).arg(&out_file.0);
            if let Some(schema) = schema {
                std::fs::write(&schema_file.0, schema.to_string())?;
                cmd.arg("--output-schema").arg(&schema_file.0);
            }
            cmd.args(&agent.args).arg("-");
            cmd::run(&mut cmd, Some(prompt))?;
            std::fs::read_to_string(&out_file.0).context("codex wrote no final message")
        }
        Tool::Claude => {
            let mut cmd = claude(agent);
            let tools = if mode == Mode::Write { CLAUDE_WRITE_TOOLS } else { CLAUDE_READ_TOOLS };
            cmd.args(["--allowedTools", tools]);
            if mode == Mode::Write {
                cmd.args(["--permission-mode", "acceptEdits"]);
            }
            if let Some(schema) = schema {
                cmd.arg("--json-schema").arg(schema.to_string());
            }
            cmd.args(&agent.args);
            claude_result(&cmd::run(&mut cmd, Some(prompt))?)
        }
    }
}

fn codex(agent: &Agent) -> Command {
    let mut cmd = Command::new("codex");
    if let Some(model) = &agent.model {
        // Top-level config override, valid for every subcommand.
        cmd.arg("-c").arg(format!("model={}", Value::from(model.as_str())));
    }
    cmd
}

fn claude(agent: &Agent) -> Command {
    let mut cmd = Command::new("claude");
    cmd.args(["-p", "--output-format", "json"]);
    if let Some(model) = &agent.model {
        cmd.args(["--model", model]);
    }
    cmd
}

#[derive(Deserialize)]
struct ClaudeOutput {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    structured_output: Option<Value>,
}

fn claude_result(stdout: &str) -> Result<String> {
    let out: ClaudeOutput = serde_json::from_str(stdout).context("unexpected output from `claude -p`")?;
    if out.is_error {
        bail!("claude failed: {}", out.result);
    }
    Ok(match out.structured_output {
        Some(value) => value.to_string(),
        None => out.result,
    })
}

/// A temp file path that is removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(suffix: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("cr-{}-{n}-{suffix}", std::process::id())))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}
