mod agent;
mod cmd;
mod comment;
mod config;
mod finding;
mod fix;
mod git;
mod host;
mod local;
mod review;
mod status;
mod target;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};

use config::{Agent, Config, Tool};
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
    /// Review local work against a base branch, then fix the findings. No PR needed;
    /// nothing is committed, pushed, or posted.
    #[command(visible_alias = "l")]
    Local {
        #[command(flatten)]
        local: LocalArgs,
        /// Only review; don't fix.
        #[arg(long)]
        no_fix: bool,
    },
    /// Repeat `local` (review, then fix) until a review finds nothing valid. Each round
    /// costs about as much as one `cr local`; nothing is committed, pushed, or posted.
    Loop {
        #[command(flatten)]
        local: LocalArgs,
        /// Stop after this many review → fix rounds.
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
        max_rounds: u32,
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

/// Options shared by `local` and `loop`.
#[derive(Args)]
struct LocalArgs {
    /// Branch or ref to compare against (default: origin's default branch, else main/master).
    #[arg(long)]
    base: Option<String>,
    /// Only run the configured reviewers using these tools.
    #[arg(long, value_delimiter = ',')]
    reviewers: Vec<Tool>,
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

    match cli.command {
        Command::Local { local, no_fix } => run_local(&config, local, !no_fix, None),
        Command::Loop { local, max_rounds } => run_local(&config, local, true, Some(max_rounds)),
        Command::Review { target, no_comment, reviewers } => {
            let reviewers = select_reviewers(&config, &reviewers)?;
            let host = host::detect(&config)?;
            let pr = checkout(host.as_ref(), target.as_deref())?;
            let opts = review::Options { comment: config.auto_comment && !no_comment, reviewers };
            review::run(&config, host.as_ref(), &pr, opts)
        }
        Command::Fix { target, no_push, no_comment } => {
            let host = host::detect(&config)?;
            let pr = checkout(host.as_ref(), target.as_deref())?;
            let opts = fix::Options { push: !no_push, comment: config.auto_comment && !no_comment };
            fix::run(&config, host.as_ref(), &pr, opts)
        }
        Command::Init { .. } => unreachable!("handled above"),
    }
}

fn run_local(config: &Config, args: LocalArgs, fix: bool, max_rounds: Option<u32>) -> Result<()> {
    let reviewers = select_reviewers(config, &args.reviewers)?;
    local::run(config, local::Options { base: args.base, fix, reviewers, max_rounds })
}

/// The configured reviewers, limited to `tools` when any are given.
fn select_reviewers(config: &Config, tools: &[Tool]) -> Result<Vec<Agent>> {
    let reviewers: Vec<_> =
        config.reviewers.iter().filter(|a| tools.is_empty() || tools.contains(&a.tool)).cloned().collect();
    if reviewers.is_empty() {
        bail!("no configured reviewer matches --reviewers");
    }
    Ok(reviewers)
}

fn checkout(host: &dyn Host, target: Option<&str>) -> Result<Pr> {
    git::ensure_clean()?;
    let pr = target::resolve(host, target)?;
    eprintln!("PR #{}: {} -> {}", pr.number, pr.head, pr.base);
    host.checkout(&pr)?;
    git::pull()?;
    Ok(pr)
}
