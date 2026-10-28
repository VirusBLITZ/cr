//! Review → fix loop on local work, before a PR exists. Nothing is committed,
//! pushed, or posted.

use anyhow::{Result, bail};

use crate::agent::Scope;
use crate::comment::{self, Changes, FixOutcome};
use crate::config::{Agent, Config};
use crate::fix;
use crate::git;
use crate::review;

pub struct Options {
    /// Ref to compare against; `None` uses the repo's default branch.
    pub base: Option<String>,
    pub fix: bool,
    pub reviewers: Vec<Agent>,
}

pub fn run(config: &Config, opts: Options) -> Result<()> {
    let base = match opts.base {
        Some(base) => base,
        None => git::default_base()?,
    };
    let before = git::changes_since(&base)?;
    if before.is_empty() {
        bail!("no changes against {base} to review");
    }

    let scope = Scope::Local { base: &base };
    let out_dir = review::out_dir("local")?;
    let (findings, used) = review::collect(config, &opts.reviewers, scope, &out_dir)?;
    let review_report = comment::render_findings(&findings, &used);
    println!("{review_report}");
    review::open_report(&out_dir, &review_report);
    if findings.is_empty() || !opts.fix {
        return Ok(());
    }

    let (items, listing) = fix::list_findings(&findings);
    let report = fix::apply(config, scope, &items, &listing)?;
    let changes = if git::changes_since(&base)? == before { Changes::None } else { Changes::Uncommitted };
    let outcome = FixOutcome { items: &items, report: &report, changes };
    let fix_report = outcome.render();
    println!("{fix_report}");
    review::open_report(&out_dir, &format!("{review_report}\n{fix_report}"));
    if !outcome.is_green() {
        bail!("not everything is green; see the summary above");
    }
    Ok(())
}
