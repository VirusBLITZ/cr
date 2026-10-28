use anyhow::{Result, bail};

use crate::host::{Host, Pr};

/// Resolves a CLI target (PR URL, PR number, branch name, or nothing for the
/// current branch) to a PR in the current repository.
pub fn resolve(host: &dyn Host, target: Option<&str>) -> Result<Pr> {
    let Some(target) = target else {
        return host.find_pr("");
    };
    if !target.contains("://") {
        return host.find_pr(target);
    }
    let Some((slug, number)) = host.parse_url(target) else {
        bail!("`{target}` is not a {} PR URL", host.name());
    };
    let current = host.repo_slug()?;
    if !slug.eq_ignore_ascii_case(&current) {
        bail!("{target} belongs to {slug}, but this repository is {current}; cd into the right repo first");
    }
    host.find_pr(&number.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Comment;

    struct Fake;

    impl Host for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn matches_remote(&self, _: &str) -> bool {
            true
        }
        fn parse_url(&self, url: &str) -> Option<(String, u64)> {
            let (slug, n) = url.strip_prefix("https://fake/")?.rsplit_once('/')?;
            Some((slug.into(), n.parse().ok()?))
        }
        fn repo_slug(&self) -> Result<String> {
            Ok("me/repo".into())
        }
        fn find_pr(&self, selector: &str) -> Result<Pr> {
            Ok(Pr { number: 1, url: String::new(), head: selector.into(), base: "main".into() })
        }
        fn checkout(&self, _: &Pr) -> Result<()> {
            unimplemented!()
        }
        fn comments(&self, _: &Pr) -> Result<Vec<Comment>> {
            unimplemented!()
        }
        fn post_comment(&self, _: &Pr, _: &str) -> Result<()> {
            unimplemented!()
        }
    }

    #[test]
    fn passes_numbers_and_branches_through() {
        assert_eq!(resolve(&Fake, None).unwrap().head, "");
        assert_eq!(resolve(&Fake, Some("12")).unwrap().head, "12");
        assert_eq!(resolve(&Fake, Some("feat/x")).unwrap().head, "feat/x");
    }

    #[test]
    fn url_must_match_current_repo() {
        assert_eq!(resolve(&Fake, Some("https://fake/Me/Repo/5")).unwrap().head, "5");
        let err = resolve(&Fake, Some("https://fake/other/repo/5")).unwrap_err();
        assert!(err.to_string().contains("belongs to other/repo"));
        assert!(resolve(&Fake, Some("https://elsewhere/x")).is_err());
    }
}
