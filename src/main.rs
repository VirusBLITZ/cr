mod agent;
mod cmd;
mod comment;
mod config;
mod finding;
mod fix;
mod git;
mod host;
mod review;
mod status;
mod target;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use config::{Config, Tool};
use host::{Host, Pr};

/// Review and fix pull requests with codex and claude.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Review a PR with every configured reviewer and post the merged findings.
    #[command(visible_alias = "r")]
    Review {
        /// PR URL, PR number, or branch name (default: the current branch's PR).
        target: Option<String>,
        /// Print the findings without posting a PR comment.
        #[arg(long)]
        no_comment: bool,
        /// Only run the configured reviewers using these tools.
        #[arg(long, value_delimiter = ',')]
        reviewers: Vec<Tool>,
    },
    /// Validate and fix review findings, push, and post a status comment.
    #[command(visible_alias = "f")]
    Fix {
        /// PR URL, PR number, or branch name (default: the current branch's PR).
        target: Option<String>,
        /// Leave fixes uncommitted (also skips the comment).
        #[arg(long)]
        no_push: bool,
        /// Don't post the status comment.
        #[arg(long)]
        no_comment: bool,
    },
    /// Write a config file with the default settings.
    #[command(visible_alias = "init-config")]
    Init {
        /// Write `.cr.toml` in the current repository instead of the user config.
        #[arg(long)]
        repo: bool,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Command::Init { repo, force } = cli.command {
        let path = if repo { git::repo_root()?.join(".cr.toml") } else { config::global_path()? };
        config::init(&path, force)?;
        println!("Wrote {}", path.display());
        return Ok(());
    }

    let root = git::repo_root()?;
    std::env::set_current_dir(&root)?;
    let config = Config::load(&root)?;
    let host = host::detect(&config)?;

    match cli.command {
        Command::Review { target, no_comment, reviewers } => {
            let reviewers: Vec<_> = config
                .reviewers
                .iter()
                .filter(|a| reviewers.is_empty() || reviewers.contains(&a.tool))
                .cloned()
                .collect();
            if reviewers.is_empty() {
                bail!("no configured reviewer matches --reviewers");
            }
            let pr = checkout(host.as_ref(), target.as_deref())?;
            let opts = review::Options { comment: config.auto_comment && !no_comment, reviewers };
            review::run(&config, host.as_ref(), &pr, opts)
        }
        Command::Fix { target, no_push, no_comment } => {
            let pr = checkout(host.as_ref(), target.as_deref())?;
            let opts = fix::Options { push: !no_push, comment: config.auto_comment && !no_comment };
            fix::run(&config, host.as_ref(), &pr, opts)
        }
        Command::Init { .. } => unreachable!("handled above"),
    }
}

fn checkout(host: &dyn Host, target: Option<&str>) -> Result<Pr> {
    git::ensure_clean()?;
    let pr = target::resolve(host, target)?;
    eprintln!("PR #{}: {} -> {}", pr.number, pr.head, pr.base);
    host.checkout(&pr)?;
    git::pull()?;
    Ok(pr)
}
