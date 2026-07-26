//! Vendored SPDX license-expression grammar + id check.
//!
//! Validates the contract's `license` field against a bundled SPDX id set so
//! license validity is deterministic and offline (no network, no pip). A direct
//! port of `check-oss-release.py`'s `spdx_valid` / `_valid_license_id`: a real
//! grammar check, not a string match.
//!
//! Grammar: `expr := term ((AND|OR) term)*` ; `term := '(' expr ')' | id
//! ['WITH' exception]`. Operators are matched case-insensitively. Exception ids
//! after `WITH` are accepted on shape (the exception list is not vendored).

/// Vendored minimal SPDX license id set — a curated mainstream OSS subset, not
/// the full 600+ list, so the membership test needs no dependency. All lowercase
/// (the check lowercases its input). Deprecated short forms (`gpl-3.0`,
/// `lgpl-2.1`, …) are included because they remain widespread in the wild.
const SPDX_LICENSE_IDS: &[&str] = &[
    "0bsd",
    "afl-3.0",
    "agpl-3.0",
    "agpl-3.0-only",
    "agpl-3.0-or-later",
    "apache-2.0",
    "artistic-2.0",
    "blueoak-1.0.0",
    "bsd-2-clause",
    "bsd-2-clause-patent",
    "bsd-3-clause",
    "bsd-3-clause-clear",
    "bsl-1.0",
    "cc-by-4.0",
    "cc-by-sa-4.0",
    "cc0-1.0",
    "cecill-2.1",
    "ecl-2.0",
    "epl-1.0",
    "epl-2.0",
    "eupl-1.1",
    "eupl-1.2",
    "gpl-2.0",
    "gpl-2.0-only",
    "gpl-2.0-or-later",
    "gpl-3.0",
    "gpl-3.0-only",
    "gpl-3.0-or-later",
    "isc",
    "lgpl-2.1",
    "lgpl-2.1-only",
    "lgpl-2.1-or-later",
    "lgpl-3.0",
    "lgpl-3.0-only",
    "lgpl-3.0-or-later",
    "mit",
    "mit-0",
    "mpl-2.0",
    "ms-pl",
    "ms-rl",
    "ncsa",
    "ofl-1.1",
    "osl-3.0",
    "postgresql",
    "python-2.0",
    "ruby",
    "unlicense",
    "upl-1.0",
    "vim",
    "wtfpl",
    "zlib",
    "zpl-2.1",
];

/// A single SPDX license id: a `LicenseRef-*`/`DocumentRef-*` custom id, or a
/// vendored id (case-insensitive), each optionally with a trailing `+`
/// ("or later").
fn valid_license_id(tok: &str) -> bool {
    let base = tok.strip_suffix('+').unwrap_or(tok);
    if base.starts_with("LicenseRef-") || base.starts_with("DocumentRef-") {
        // Custom ref: shape-only check (no membership). Non-empty, and every
        // char is alphanumeric or one of `.`/`-`/`:` (the Python regex).
        return !base.is_empty()
            && base
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    }
    let lower = base.to_ascii_lowercase();
    SPDX_LICENSE_IDS.contains(&lower.as_str())
}

/// Whether `expr` is a syntactically valid SPDX license expression whose license
/// ids are all recognized.
#[must_use]
pub fn spdx_valid(expr: &str) -> bool {
    if expr.trim().is_empty() {
        return false;
    }
    // Tokenize: parenthesess split off as their own tokens, then whitespace-split.
    let spaced = expr.replace('(', " ( ").replace(')', " ) ");
    let toks: Vec<&str> = spaced.split_whitespace().collect();
    let mut parser = Parser {
        toks: &toks,
        pos: 0,
    };
    parser.parse_expr() && parser.pos == parser.toks.len()
}

struct Parser<'a> {
    toks: &'a [&'a str],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.toks.get(self.pos).copied()
    }

    fn is_op(tok: Option<&str>) -> bool {
        matches!(tok, Some(t) if t.eq_ignore_ascii_case("AND") || t.eq_ignore_ascii_case("OR"))
    }

    fn parse_expr(&mut self) -> bool {
        if !self.parse_term() {
            return false;
        }
        while Self::is_op(self.peek()) {
            self.pos += 1;
            if !self.parse_term() {
                return false;
            }
        }
        true
    }

    fn parse_term(&mut self) -> bool {
        let Some(t) = self.peek() else {
            return false;
        };
        if t == "(" {
            self.pos += 1;
            if !self.parse_expr() {
                return false;
            }
            if self.peek() != Some(")") {
                return false;
            }
            self.pos += 1;
            return true;
        }
        // A term must start with a license id: not a paren, operator, or WITH.
        if t == "(" || t == ")" || Self::is_op(Some(t)) || t.eq_ignore_ascii_case("WITH") {
            return false;
        }
        self.pos += 1; // consume the license id
        if !valid_license_id(t) {
            return false;
        }
        if matches!(self.peek(), Some(w) if w.eq_ignore_ascii_case("WITH")) {
            self.pos += 1;
            // The exception id is accepted on shape (not vendored).
            match self.peek() {
                Some(exc)
                    if exc != "("
                        && exc != ")"
                        && !Self::is_op(Some(exc))
                        && !exc.eq_ignore_ascii_case("WITH") =>
                {
                    self.pos += 1;
                }
                _ => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::spdx_valid;

    #[test]
    fn simple_ids() {
        assert!(spdx_valid("MIT"));
        assert!(spdx_valid("Apache-2.0"));
        assert!(spdx_valid("mit")); // case-insensitive
        assert!(spdx_valid("GPL-3.0-only"));
    }

    #[test]
    fn or_later_suffix() {
        assert!(spdx_valid("GPL-3.0+"));
        assert!(spdx_valid("Apache-2.0+"));
    }

    #[test]
    fn compound_expressions() {
        assert!(spdx_valid("MIT OR Apache-2.0"));
        assert!(spdx_valid("MIT AND Apache-2.0"));
        assert!(spdx_valid("(MIT OR Apache-2.0) AND ISC"));
        assert!(spdx_valid("mit or apache-2.0")); // operators case-insensitive
    }

    #[test]
    fn with_exception() {
        assert!(spdx_valid("GPL-3.0-only WITH Classpath-exception-2.0"));
        assert!(!spdx_valid("GPL-3.0-only WITH")); // dangling WITH
    }

    #[test]
    fn custom_refs() {
        assert!(spdx_valid("LicenseRef-Acme-Proprietary"));
        assert!(spdx_valid("DocumentRef-x:LicenseRef-y"));
    }

    #[test]
    fn rejects_unknown_and_malformed() {
        assert!(!spdx_valid(""));
        assert!(!spdx_valid("   "));
        assert!(!spdx_valid("Proprietary-Acme")); // unknown id
        assert!(!spdx_valid("MIT AND")); // dangling operator
        assert!(!spdx_valid("(MIT")); // unbalanced paren
        assert!(!spdx_valid("MIT OR OR Apache-2.0")); // double operator
        assert!(!spdx_valid("MIT Apache-2.0")); // missing operator
        assert!(!spdx_valid("()")); // empty group
    }
}
