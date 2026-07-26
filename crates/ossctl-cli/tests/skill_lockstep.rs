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
//!   3. every `ossctl …` command in a fenced code block resolves to a real
//!      subcommand path, and every `--flag` it references appears in that
//!      subcommand's `--help`.
//!
//! It is deliberately a *runtime* check against the actual binary rather than a
//! hand-maintained list: when the CLI surface changes, this test — not a
//! production incident — catches a skill that drifted.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

fn ossctl() -> Command {
    Command::cargo_bin("ossctl").expect("ossctl binary builds")
}

fn skills_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills")
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

        // The installed/printed body has no unrendered token left.
        let rendered = ossctl()
            .args(["skill", "print", &name])
            .output()
            .unwrap()
            .stdout;
        let rendered = String::from_utf8(rendered).unwrap();
        assert!(
            !rendered.contains("{{"),
            "{name}: rendered skill still contains an unrendered token"
        );
    }
}

/// (3): every `ossctl …` command in a fenced block resolves against the binary.
#[test]
fn referenced_commands_and_flags_exist() {
    for (name, path) in bundled_templates() {
        let raw = std::fs::read_to_string(&path).unwrap();
        for cmd in fenced_ossctl_commands(&raw) {
            check_command(&name, &cmd);
        }
    }
}

/// Self-guard: the gate must reject a command that names a flag the CLI lacks,
/// otherwise a green run proves nothing.
#[test]
#[should_panic(expected = "unknown flag")]
fn gate_rejects_a_bogus_flag() {
    check_command("self-test", "ossctl audit --no-such-flag");
}

/// Self-guard: the gate must reject an unknown subcommand path too.
#[test]
#[should_panic(expected = "unknown subcommand")]
fn gate_rejects_a_bogus_subcommand() {
    check_command("self-test", "ossctl frobnicate widgets");
}

// ── extraction + validation ──────────────────────────────────────────────────

/// Pull every `ossctl …` invocation out of the fenced code blocks. Prose
/// (inline backticks) is not validated — only executable example blocks are,
/// mirroring how the sibling CLIs gate their skills.
fn fenced_ossctl_commands(text: &str) -> Vec<String> {
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
        if let Some(rest) = line.strip_prefix("ossctl ") {
            // Cut the shell tail: `|| exit`, pipes, `> redirect`, `&&`. `<` is
            // NOT a cut point — it opens `<placeholder>` positional args, which
            // the validator handles as non-flag tokens further down.
            let end = rest
                .find(&['|', '>'][..])
                .or_else(|| rest.find("&&"))
                .unwrap_or(rest.len());
            let cmd = format!("ossctl {}", rest[..end].trim());
            cmds.push(cmd);
        }
    }
    cmds
}

/// Resolve one `ossctl …` command against the live binary: find the longest
/// leading token run that is a real subcommand path, then verify every `--flag`
/// it references is listed in that subcommand's `--help`.
fn check_command(skill: &str, cmd: &str) {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    assert_eq!(
        tokens[0], "ossctl",
        "{skill}: `{cmd}` must start with ossctl"
    );
    let args = &tokens[1..];

    // Candidate path = leading non-flag tokens (positional placeholders like
    // `<run_id>` or a skill name are trimmed away by the shrink loop below).
    let candidate_len = args.iter().take_while(|t| !t.starts_with('-')).count();

    // Shrink from the longest candidate until `ossctl <path> --help` succeeds.
    // This strips trailing positional args (`skill print oss-release` → `skill
    // print`) without needing to know the arity of each verb.
    let mut help = None;
    for len in (1..=candidate_len).rev() {
        let path = &args[..len];
        let out = ossctl().args(path).arg("--help").output().unwrap();
        if out.status.success() {
            help = Some((
                path.to_vec(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
            ));
            break;
        }
    }
    let (path, help_text) = help.unwrap_or_else(|| {
        panic!("{skill}: `{cmd}` references an unknown subcommand (no valid path prefix)")
    });

    // Every referenced long flag must appear in that subcommand's help.
    for tok in args {
        if let Some(flag) = tok.strip_prefix("--") {
            let flag = flag.split('=').next().unwrap();
            if flag.is_empty() {
                continue;
            }
            assert!(
                help_text.contains(&format!("--{flag}")),
                "{skill}: `{cmd}` references unknown flag `--{flag}` on `ossctl {}`",
                path.join(" ")
            );
        }
    }
}

/// Read a scalar `key:` value out of the leading YAML frontmatter block,
/// unquoted. Mirrors the binary's own hand-rolled frontmatter reader.
fn frontmatter_field(text: &str, key: &str) -> Option<String> {
    let body = text.strip_prefix("---")?;
    let end = body.find("\n---")?;
    for line in body[..end].lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            let v = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}
