//! §17 skill↔CLI lockstep gate.
//!
//! The binary is the source of truth; a bundled skill that references a
//! subcommand or flag the CLI does not expose is a release-blocker, exactly as
//! a broken `--help` would be. This test mechanically verifies, for every
//! bundled `SKILL.template.md`:
//!
//!   1. its frontmatter `name` matches its directory,
//!   2. it carries the `{{CLI_VERSION}}` token (so `print`/`install` can pin it)
//!      and no `{{…}}` token survives rendering, and
//!   3. every `shipshape …` command in a fenced code block resolves to a real
//!      subcommand path, and every `--flag` it references appears in that
//!      subcommand's `--help`.
//!
//! It is deliberately a *runtime* check against the actual binary rather than a
//! hand-maintained list: when the CLI surface changes, this test — not a
//! production incident — catches a skill that drifted.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

fn shipshape() -> Command {
    Command::cargo_bin("shipshape").expect("shipshape binary builds")
}

fn skills_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills")
}

fn repo_root() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent()?.parent()?;
    (root.join("Cargo.toml").is_file() && root.join("OSS-RELEASE.md").is_file())
        .then(|| root.to_path_buf())
}

/// Every `skills/<name>/SKILL.template.md` on disk.
fn bundled_templates() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(skills_dir())
        .expect("skills/ dir exists")
        .flatten()
    {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let template = dir.join("SKILL.template.md");
        if template.exists() {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, template));
        }
    }
    out.sort();
    assert!(!out.is_empty(), "at least one skill must be bundled");
    out
}

/// (1) + (2): frontmatter name matches directory; the version token is present
/// in the raw template and fully substituted after rendering.
#[test]
fn frontmatter_is_well_formed_and_pins_version() {
    for (name, path) in bundled_templates() {
        let raw = std::fs::read_to_string(&path).unwrap();

        // Raw template must carry the substitution token (else `print`/`install`
        // would ship an unpinned skill).
        assert!(
            raw.contains("{{CLI_VERSION}}"),
            "{name}: template must carry the {{{{CLI_VERSION}}}} token"
        );

        // Frontmatter `name:` matches the directory.
        let fm_name = frontmatter_field(&raw, "name")
            .unwrap_or_else(|| panic!("{name}: no `name:` in frontmatter"));
        assert_eq!(fm_name, name, "{name}: frontmatter name matches directory");

        // `skill print` must SUCCEED for this template — a template on disk that
        // is not in the binary's catalog would print nothing (empty stdout) and
        // silently pass the token check below. Assert exit 0 first.
        let out = shipshape()
            .args(["skill", "print", &name])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{name}: template exists on disk but is not printable (missing from CATALOG?): {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // No *substitution token* survives rendering. Checked specifically, not
        // as a blanket `{{` ban, so a template may legitimately show e.g. a
        // GitHub Actions `${{ secrets.X }}` snippet.
        let rendered = String::from_utf8(out.stdout).unwrap();
        for token in ["{{CLI_VERSION}}", "{{SKILL_SCHEMA_VERSION}}"] {
            assert!(
                !rendered.contains(token),
                "{name}: rendered skill still contains {token}"
            );
        }
    }
}

/// The on-disk template set, the binary's `skill list` catalog, and safe-slug
/// naming must all agree — no orphan template, no catalog entry whose name could
/// escape the install root (`../`, absolute, separators).
#[test]
fn catalog_matches_disk_and_names_are_safe_slugs() {
    let mut on_disk: Vec<String> = bundled_templates().into_iter().map(|(n, _)| n).collect();
    on_disk.sort();

    let out = shipshape()
        .args(["skill", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut catalog: Vec<String> = v["data"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();
    catalog.sort();

    assert_eq!(
        on_disk, catalog,
        "every bundled template must be in the catalog and vice-versa"
    );

    for name in &catalog {
        assert!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && !name.starts_with('-'),
            "skill name `{name}` is not a safe slug (would risk path escape under the install root)"
        );
    }
}

/// (3): every `shipshape …` command in a fenced block resolves against the binary.
#[test]
fn referenced_commands_and_flags_exist() {
    for (name, path) in bundled_templates() {
        let raw = std::fs::read_to_string(&path).unwrap();
        for cmd in fenced_shipshape_commands(&raw) {
            check_command(&name, &cmd);
        }
    }
}

/// The registry package and cargo-dist wrapper deliberately share a binary name,
/// so repository automation must select the package coordinate, not the product
/// coordinate. This also runs in the workspace-wide test job, independently of
/// the named lockstep job whose selector it protects.
#[test]
fn repository_workflows_select_the_registry_cli_package() {
    let Some(root) = repo_root() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must run this repository guard from a complete source checkout"
        );
        return;
    };

    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("source checkout must track the CI workflow");
    let lockstep = ci
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.starts_with('#')
                && line.contains("cargo test")
                && line.contains("--test skill")
                && line.contains("--test skill_lockstep")
        })
        .expect("the named lockstep job must run both skill integration tests");
    assert!(
        command_selects_package(lockstep, "shipshape-cli"),
        "the lockstep job must select Cargo package `shipshape-cli`: {lockstep}"
    );
    assert!(
        !command_selects_package(lockstep, "shipshape"),
        "the lockstep job must not select the cargo-dist wrapper: {lockstep}"
    );

    let publish = std::fs::read_to_string(root.join(".github/workflows/publish-crates.yml"))
        .expect("source checkout must track the crates.io publish workflow");
    assert!(
        !publish.contains("core_publishable") && !publish.contains("Skipping shipshape-core"),
        "the one-time recovery gate must not remain in the steady-state workflow"
    );
    assert!(
        publish.contains("cargo package --locked -p shipshape-cli --no-verify"),
        "dry-run dispatch must package-check the CLI before any irreversible upload"
    );
    let mut publish_calls = Vec::new();
    for line in publish.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with("publish()") {
            continue;
        }
        let tokens = shellish_tokens(line);
        for pair in tokens.windows(2) {
            if matches!(pair[0].as_str(), "-p" | "--package") {
                assert_ne!(
                    pair[1], "shipshape",
                    "crates.io workflow selects the non-published cargo-dist wrapper: {line}"
                );
            }
        }
        let helper_arg = match tokens.as_slice() {
            [verb, package, ..] if verb == "publish" => Some(package),
            [condition, verb, package, ..] if condition == "if" && verb == "publish" => {
                Some(package)
            }
            _ => None,
        };
        if let Some(package) = helper_arg {
            assert!(
                matches!(package.as_str(), "shipshape-core" | "shipshape-cli"),
                "publish helper calls must use a literal registry package: {line}"
            );
            publish_calls.push(package.clone());
        }
    }
    assert_eq!(
        publish_calls,
        vec!["shipshape-core", "shipshape-cli"],
        "the workflow script must contain one helper call per registry package, core first"
    );
}

fn shellish_tokens(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| matches!(c, '\'' | '"' | ';' | '(' | ')' | ','))
                .to_string()
        })
        .collect()
}

fn command_selects_package(command: &str, package: &str) -> bool {
    let tokens = shellish_tokens(command);
    tokens
        .windows(2)
        .any(|pair| matches!(pair[0].as_str(), "-p" | "--package") && pair[1] == package)
        || tokens.iter().any(|token| {
            token == &format!("-p={package}") || token == &format!("--package={package}")
        })
}

/// Self-guard: the gate must reject a command that names a flag the CLI lacks,
/// otherwise a green run proves nothing.
#[test]
#[should_panic(expected = "unknown flag")]
fn gate_rejects_a_bogus_flag() {
    check_command("self-test", "shipshape audit --no-such-flag");
}

/// Self-guard: the gate must reject an unknown top-level subcommand path.
#[test]
#[should_panic(expected = "unknown subcommand")]
fn gate_rejects_a_bogus_subcommand() {
    check_command("self-test", "shipshape frobnicate widgets");
}

/// Self-guard for the nastiest false-negative: a bogus CHILD of a real parent
/// verb. `release` is real but `frobnicate` is not — the gate must not silently
/// treat `frobnicate` as an ignored positional and validate against `release`.
#[test]
#[should_panic(expected = "unknown subcommand")]
fn gate_rejects_a_bogus_child_of_a_real_parent() {
    check_command("self-test", "shipshape release frobnicate --json");
}

/// Self-guard: a flag that is a *prefix* of a real flag must be rejected — the
/// help match is boundary-aware, so `--pla` does not pass on `--plan`.
#[test]
#[should_panic(expected = "unknown flag")]
fn gate_rejects_a_prefix_of_a_real_flag() {
    check_command("self-test", "shipshape release cut --pla");
}

// ── extraction + validation ──────────────────────────────────────────────────

/// Pull every `shipshape …` invocation out of the fenced code blocks. Prose
/// (inline backticks) is not validated — only executable example blocks are,
/// mirroring how the sibling CLIs gate their skills.
fn fenced_shipshape_commands(text: &str) -> Vec<String> {
    let mut cmds = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            continue;
        }
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("shipshape ") {
            // Cut the shell tail: `|| exit`, pipes, `> redirect`, `&&`. `<` is
            // NOT a cut point — it opens `<placeholder>` positional args, which
            // the validator handles as non-flag tokens further down.
            let end = rest
                .find(&['|', '>'][..])
                .or_else(|| rest.find("&&"))
                .unwrap_or(rest.len());
            let cmd = format!("shipshape {}", rest[..end].trim());
            cmds.push(cmd);
        }
    }
    cmds
}

/// Resolve one `shipshape …` command against the live binary, then verify every
/// referenced flag exists.
///
/// The subcommand path is the leading run of tokens that are neither flags
/// (`-…`) nor positional placeholders (`<…>`) — e.g. `contract show` in
/// `contract show --json`, `skill print shipshape-release` in `skill print
/// shipshape-release`. That exact path must resolve (`shipshape <path> --help` exits 0);
/// we do NOT shrink on failure, because shrinking would silently accept a bogus
/// child of a real parent (`release frobnicate` → validated against `release`).
fn check_command(skill: &str, cmd: &str) {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    assert_eq!(
        tokens[0], "shipshape",
        "{skill}: `{cmd}` must start with shipshape"
    );
    let args = &tokens[1..];

    let path_len = args
        .iter()
        .take_while(|t| !t.starts_with('-') && !t.starts_with('<'))
        .count();
    let path = &args[..path_len];

    let out = shipshape().args(path).arg("--help").output().unwrap();
    assert!(
        out.status.success(),
        "{skill}: `{cmd}` references an unknown subcommand `shipshape {}`",
        path.join(" ")
    );
    let help_text = String::from_utf8_lossy(&out.stdout).into_owned();

    // Every referenced long flag must be a real flag on that subcommand. The
    // match is boundary-aware (token-exact), so `--force` is NOT satisfied by a
    // help entry for `--force-approved`.
    for tok in args {
        if let Some(flag) = tok.strip_prefix("--") {
            let flag = flag.split('=').next().unwrap();
            if flag.is_empty() {
                continue;
            }
            assert!(
                help_lists_flag(&help_text, flag),
                "{skill}: `{cmd}` references unknown flag `--{flag}` on `shipshape {}`",
                path.join(" ")
            );
        }
    }
}

/// Whether `--help` output declares `--<flag>` as an actual flag (not merely a
/// substring of a longer flag or of prose). clap renders flags as whitespace-
/// separated tokens like `--json`, `--plan`, `-h,`, `--help`; a value hint
/// follows as a separate `<VALUE>` token. So a token-exact comparison (after
/// trimming a trailing `,`) is a reliable boundary check without a regex dep.
fn help_lists_flag(help: &str, flag: &str) -> bool {
    let want = format!("--{flag}");
    help.split_whitespace()
        .any(|t| t.trim_end_matches(',') == want)
}

/// Read a top-level scalar `key:` value out of the leading YAML frontmatter
/// block, unquoted. Mirrors the binary's own hardened reader (BOM tolerant,
/// top-level keys only, trailing `# comment` stripped) so the gate and the
/// binary agree on what a frontmatter value is.
fn frontmatter_field(text: &str, key: &str) -> Option<String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let body = text.strip_prefix("---")?;
    let end = body.find("\n---")?;
    for line in body[..end].lines() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(rest) = line.trim_end().strip_prefix(&format!("{key}:")) else {
            continue;
        };
        let v = rest.split(" #").next().unwrap_or(rest).trim();
        let v = v.trim_matches(|c| c == '"' || c == '\'');
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}
