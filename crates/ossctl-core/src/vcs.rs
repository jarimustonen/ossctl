//! Version-control helpers shared across domains — the neutral home for git
//! remote/URL parsing that both the readiness [audit](crate::audit) and the
//! release [coordinator](crate::release::coordinator) depend on.
//!
//! Kept out of any single domain module so the dependency runs one way (each
//! consumer → `vcs`) rather than one domain reaching into another's internals.

/// Every recognized way a GitHub remote URL prefixes the `owner/repo` tail. The
/// match is a **prefix**, not a substring, so a non-GitHub host that merely
/// contains `github.com` in its path (`https://mirror.example/github.com/o/r`)
/// is rejected rather than mis-parsed into a GitHub slug.
const GITHUB_PREFIXES: &[&str] = &[
    "git@github.com:",       // scp-like (the common SSH form)
    "ssh://git@github.com/", // explicit ssh:// SSH form
    "ssh://github.com/",     // ssh:// without a user
    "https://github.com/",   //
    "http://github.com/",    //
    "git://github.com/",     //
    "github.com:",           // bare scp-like
    "github.com/",           // bare
];

/// Parse `owner/repo` out of a GitHub remote URL — the SSH (`git@github.com:o/r`,
/// `ssh://git@github.com/o/r`), HTTPS, and `git://` forms. `None` for a
/// non-GitHub host or an unrecognizable URL. The parser anchors on a known host
/// prefix (never a bare `find`), so a lookalike host is never accepted; a wrong
/// parse would in any case only yield a failed `gh api` ⇒ `unknown`, never a
/// false `Absent`.
pub(crate) fn parse_github_slug(url: &str) -> Option<String> {
    let tail = GITHUB_PREFIXES.iter().find_map(|p| url.strip_prefix(p))?;
    // Trim a trailing slash BEFORE stripping `.git` so `.../repo.git/` and
    // `.../repo/` both reduce to `repo` (strip order matters).
    let tail = tail.trim_end_matches('/');
    let tail = tail.strip_suffix(".git").unwrap_or(tail);
    let tail = tail.trim_end_matches('/');
    let mut parts = tail.splitn(3, '/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    // Reject a trailing path segment (`owner/repo/extra`) — not a bare slug.
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::parse_github_slug;

    #[test]
    fn parses_github_slugs_across_url_forms() {
        assert_eq!(
            parse_github_slug("git@github.com:acme/tool.git"),
            Some("acme/tool".to_string())
        );
        assert_eq!(
            parse_github_slug("https://github.com/acme/tool.git"),
            Some("acme/tool".to_string())
        );
        assert_eq!(
            parse_github_slug("https://github.com/acme/tool"),
            Some("acme/tool".to_string())
        );
        assert_eq!(
            parse_github_slug("git://github.com/acme/tool.git"),
            Some("acme/tool".to_string())
        );
        assert_eq!(
            parse_github_slug("ssh://git@github.com/acme/tool.git"),
            Some("acme/tool".to_string())
        );
        // A trailing slash after `.git` still reduces to the bare slug.
        assert_eq!(
            parse_github_slug("https://github.com/acme/tool.git/"),
            Some("acme/tool".to_string())
        );
        assert_eq!(
            parse_github_slug("https://github.com/acme/tool/"),
            Some("acme/tool".to_string())
        );

        // Non-GitHub host, lookalike hosts, and over-long paths are rejected.
        assert_eq!(parse_github_slug("git@gitlab.com:acme/tool.git"), None);
        assert_eq!(
            parse_github_slug("https://mirror.example.com/github.com/acme/tool.git"),
            None
        );
        assert_eq!(
            parse_github_slug("https://github.com.evil.example/acme/tool"),
            None
        );
        assert_eq!(
            parse_github_slug("https://github.com/acme/tool/tree/main"),
            None
        );
        assert_eq!(parse_github_slug("https://github.com/acme"), None);
    }
}
