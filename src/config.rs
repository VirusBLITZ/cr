use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Codex,
    Claude,
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Tool::Codex => "codex",
            Tool::Claude => "claude",
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Agent {
    pub tool: Tool,
    /// Model passed to the CLI; `None` uses the CLI's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Extra arguments appended verbatim to every invocation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl Agent {
    pub fn new(tool: Tool) -> Self {
        Self { tool, model: None, args: Vec::new() }
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.model {
            Some(model) => write!(f, "{} ({model})", self.tool),
            None => write!(f, "{}", self.tool),
        }
    }
}

// Scalars come before agents so the struct serializes as valid TOML.
#[derive(Serialize, Debug, PartialEq)]
pub struct Config {
    pub auto_comment: bool,
    /// Git host name (e.g. "github"); `None` auto-detects from the origin remote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub reviewers: Vec<Agent>,
    pub merger: Agent,
    pub fixer: Agent,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reviewers: vec![Agent::new(Tool::Codex), Agent::new(Tool::Claude)],
            merger: Agent { model: Some("gpt-6-luna".into()), ..Agent::new(Tool::Codex) },
            fixer: Agent::new(Tool::Claude),
            auto_comment: true,
            host: None,
        }
    }
}

/// One config file; every field is optional so files can be layered.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Layer {
    reviewers: Option<Vec<Agent>>,
    merger: Option<Agent>,
    fixer: Option<Agent>,
    auto_comment: Option<bool>,
    host: Option<String>,
}

impl Config {
    /// Defaults, overlaid by `~/.config/cr/config.toml`, overlaid by `<repo>/.cr.toml`.
    pub fn load(repo_root: &Path) -> Result<Self> {
        let mut config = Self::default();
        let global = global_path().ok();
        for path in global.iter().chain([&repo_root.join(".cr.toml")]) {
            if let Ok(text) = std::fs::read_to_string(path) {
                let layer = toml::from_str(&text)
                    .with_context(|| format!("invalid config {}", path.display()))?;
                config.apply(layer);
            }
        }
        Ok(config)
    }

    fn apply(&mut self, layer: Layer) {
        if let Some(v) = layer.reviewers {
            self.reviewers = v;
        }
        if let Some(v) = layer.merger {
            self.merger = v;
        }
        if let Some(v) = layer.fixer {
            self.fixer = v;
        }
        if let Some(v) = layer.auto_comment {
            self.auto_comment = v;
        }
        if layer.host.is_some() {
            self.host = layer.host;
        }
    }
}

const TEMPLATE_HEADER: &str = "\
# cr configuration. A repository's .cr.toml overrides this file key by key.
#
# Each agent has a `tool` (codex | claude), an optional `model` (omit it to use
# the CLI's own default) and optional `args` passed verbatim to the CLI.
# Set `host` (e.g. \"github\") to skip detecting the git host from the origin remote.

";

pub fn global_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|d| d.join("cr").join("config.toml"))
        .context("could not determine the user config directory")
}

/// Writes the default config to `path`, refusing to overwrite unless `force`.
pub fn init(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!("{} already exists; pass --force to overwrite it", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, template()).with_context(|| format!("could not write {}", path.display()))
}

fn template() -> String {
    let defaults = toml::to_string(&Config::default()).expect("default config serializes");
    format!("{TEMPLATE_HEADER}{defaults}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_overrides_only_given_fields() {
        let mut config = Config::default();
        config.apply(
            toml::from_str(
                r#"
                auto_comment = false
                [[reviewers]]
                tool = "claude"
                model = "opus"
                args = ["--verbose"]
                "#,
            )
            .unwrap(),
        );
        assert!(!config.auto_comment);
        assert_eq!(
            config.reviewers,
            vec![Agent { tool: Tool::Claude, model: Some("opus".into()), args: vec!["--verbose".into()] }]
        );
        assert_eq!(config.merger, Config::default().merger);
    }

    #[test]
    fn template_parses_back_to_defaults() {
        let mut config = Config { auto_comment: false, reviewers: vec![], ..Config::default() };
        config.apply(toml::from_str(&template()).unwrap());
        assert_eq!(config, Config::default());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Layer>("revewers = []").is_err());
    }
}
