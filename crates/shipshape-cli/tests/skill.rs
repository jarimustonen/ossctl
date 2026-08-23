//! Behavioral tests for the `shipshape skill` subcommand (`AGENTS-AI-FIRST-CLI.md`
//! §15–§17): `list`, `print`, and `install` (token substitution + drift policy).

use assert_cmd::Command;
use predicates::prelude::*;

fn shipshape() -> Command {
    Command::cargo_bin("shipshape").expect("shipshape binary builds")
}

/// The binary's own version — bundled skills pin to it (§17).
fn cli_version() -> String {
    let out = shipshape().args(["version", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["data"]["version"].as_str().unwrap().to_string()
}

// ── skill list ───────────────────────────────────────────────────────────────

/// `skill list --json` enumerates the catalog with §17 version fields, and every
/// entry's `cli_version` equals the running binary's version.
#[test]
fn skill_list_json_carries_version_fields() {
    let ver = cli_version();
    let out = shipshape()
        .args(["skill", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema_version"], 1);
    let skills = v["data"]["skills"].as_array().expect("skills array");
    let names: Vec<&str> = skills
        .iter()
        .map(|skill| skill["name"].as_str().expect("skill name"))
        .collect();
    assert_eq!(
        names,
        vec![
            "shipshape-architecture",
            "shipshape-init",
            "shipshape-release",
            "shipshape-ci",
            "shipshape-security-policy",
            "shipshape-changelog",
            "shipshape-readiness",
            "shipshape-readme",
            "shipshape-contributing",
            "shipshape-dist",
        ],
        "the catalog contains exactly the ten canonical Shipshape skills"
    );
    for s in skills {
        assert!(s["name"].is_string(), "name present: {s}");
        assert!(s["description"].is_string(), "description present: {s}");
        assert_eq!(s["cli_version"], ver, "cli_version pinned to binary: {s}");
        assert_eq!(s["schema_version"], 1, "skill schema_version: {s}");
        assert!(s["path_in_repo"].is_string(), "path_in_repo present: {s}");
    }
}

/// `version --json` exposes the same catalog (§17: one call to audit freshness).
#[test]
fn version_json_lists_bundled_skills() {
    let list = shipshape()
        .args(["skill", "list", "--json"])
        .output()
        .unwrap();
    let lv: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let names_from_list: Vec<String> = lv["data"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();

    let ver = shipshape().args(["version", "--json"]).output().unwrap();
    let vv: serde_json::Value = serde_json::from_slice(&ver.stdout).unwrap();
    let names_from_version: Vec<String> = vv["data"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(
        names_from_list, names_from_version,
        "version and skill list expose the same catalog"
    );
}

#[test]
fn legacy_skill_name_is_refused_with_migration_guidance() {
    shipshape()
        .args(["skill", "print", "oss-release", "--json"])
        .assert()
        .failure()
        .code(1)
        .stderr(
            predicate::str::contains("\"code\":\"skill_renamed\"")
                .and(predicate::str::contains("shipshape-release"))
                .and(predicate::str::contains("shipshape skill install")),
        );
}

#[test]
fn every_skill_body_uses_only_canonical_command_and_skill_names() {
    let list = shipshape()
        .args(["skill", "list", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    for skill in value["data"]["skills"].as_array().unwrap() {
        let name = skill["name"].as_str().unwrap();
        let printed = shipshape().args(["skill", "print", name]).output().unwrap();
        assert!(printed.status.success(), "{name} prints");
        let body = String::from_utf8(printed.stdout).unwrap();
        assert!(
            !body.contains("ossctl") && !body.contains("/oss-"),
            "{name} contains a retired command, crate, or skill reference"
        );
    }
}

// ── skill print ──────────────────────────────────────────────────────────────

/// `skill print` streams the resolved SKILL.md with tokens substituted for the
/// running binary's version and no `{{…}}` placeholder left behind.
#[test]
fn skill_print_substitutes_tokens() {
    let ver = cli_version();
    let out = shipshape()
        .args(["skill", "print", "shipshape-release"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for token in ["{{CLI_VERSION}}", "{{SKILL_SCHEMA_VERSION}}"] {
        assert!(
            !text.contains(token),
            "unrendered token {token} survives print: {text}"
        );
    }
    assert!(
        text.contains(&format!("cli_version: \"{ver}\"")),
        "frontmatter pins the running version"
    );
}

/// `skill print --json` routes body and metadata separately (§16).
#[test]
fn skill_print_json_shape() {
    let ver = cli_version();
    let out = shipshape()
        .args(["skill", "print", "shipshape-release", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let data = &v["data"];
    assert_eq!(data["name"], "shipshape-release");
    assert_eq!(data["cli_version"], ver);
    assert_eq!(data["schema_version_skill"], 1);
    assert!(data["content"]
        .as_str()
        .unwrap()
        .contains("# /shipshape-release"));
    assert!(data["path_in_repo"].is_string());
}

/// `skill print` text is byte-identical to what `install` writes to disk (§16).
#[test]
fn skill_print_matches_installed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    shipshape()
        .args(["skill", "install", "shipshape-release", "--dest"])
        .arg(dir.path())
        .assert()
        .success();
    let installed = std::fs::read(dir.path().join("shipshape-release/SKILL.md")).unwrap();

    let printed = shipshape()
        .args(["skill", "print", "shipshape-release"])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(installed, printed, "install == print (§16)");
}

/// Unknown skill → §10 error envelope carrying the accepted set, exit 1.
#[test]
fn skill_print_unknown_is_structured_error() {
    let out = shipshape()
        .args(["skill", "print", "does-not-exist"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "unknown_skill");
    assert_eq!(v["error"]["invalid_value"], "does-not-exist");
    assert!(v["error"]["expected"].is_array(), "accepted set present");
}

/// Golden `shipshape-init`: it prints, pins the running version, resolves cleanly, and
/// shells out to the binary subcommands (never the retired Python scripts).
#[test]
fn skill_print_shipshape_init_is_wired_to_the_binary() {
    let ver = cli_version();
    let out = shipshape()
        .args(["skill", "print", "shipshape-init"])
        .output()
        .unwrap();
    assert!(out.status.success(), "shipshape-init prints");
    let text = String::from_utf8(out.stdout).unwrap();

    // Pinned + fully rendered.
    for token in ["{{CLI_VERSION}}", "{{SKILL_SCHEMA_VERSION}}"] {
        assert!(!text.contains(token), "unrendered token {token}: {text}");
    }
    assert!(
        text.contains(&format!("cli_version: \"{ver}\"")),
        "frontmatter pins the running version"
    );
    assert!(text.contains("# /shipshape-init"), "body heading present");

    // The migration's whole point: shells out to the binary, not Python. All
    // three subcommands the lifecycle depends on must be present.
    for sub in [
        "shipshape facts",
        "shipshape contract show",
        "shipshape contract validate",
    ] {
        assert!(text.contains(sub), "shipshape-init must invoke `{sub}`");
    }
    for retired in ["infer-repo-facts.py", "check-oss-release.py", "python3"] {
        assert!(
            !text.contains(retired),
            "shipshape-init must not reference the retired `{retired}`"
        );
    }
}

/// `shipshape-dist` is a registered family member and installs the exact rendered
/// distribution-channel operating manual into an explicit destination (§16–§17).
#[test]
fn skill_install_shipshape_dist_writes_distribution_manual() {
    let dir = tempfile::tempdir().unwrap();
    shipshape()
        .args([
            "skill",
            "install",
            "shipshape-dist",
            "--agent",
            "claude",
            "--dest",
        ])
        .arg(dir.path())
        .assert()
        .success();

    let installed = std::fs::read_to_string(dir.path().join("shipshape-dist/SKILL.md")).unwrap();
    assert!(installed.contains("# /shipshape-dist"));
    assert!(installed.contains("shipshape dist generate"));
    assert!(installed.contains("HOMEBREW_TAP_TOKEN"));
    for target in [
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
    ] {
        assert!(
            installed.contains(target),
            "cross-platform target is documented: {target}"
        );
    }
    assert!(installed.contains("Intel macOS"));
    assert!(
        !installed.contains("x86_64-apple-darwin"),
        "the maintained skill must not list the withdrawn Intel target"
    );
    for forbidden in [
        "builder-host",
        "github-custom-runners",
        "cargo install cargo-dist",
    ] {
        assert!(
            !installed.contains(forbidden),
            "skill must not instruct personal infrastructure or global installation: {forbidden}"
        );
    }
    assert!(!installed.contains("{{CLI_VERSION}}"));
}

/// `shipshape-init` install is byte-identical to print (§16) and lands the canonical file.
#[test]
fn skill_install_shipshape_init_matches_print() {
    let dir = tempfile::tempdir().unwrap();
    shipshape()
        .args(["skill", "install", "shipshape-init", "--dest"])
        .arg(dir.path())
        .assert()
        .success();
    let installed = std::fs::read(dir.path().join("shipshape-init/SKILL.md")).unwrap();

    let printed = shipshape()
        .args(["skill", "print", "shipshape-init"])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(installed, printed, "install == print (§16)");
}

// ── skill install ────────────────────────────────────────────────────────────

/// A clean single-runtime install writes the canonical file and reports it in the
/// envelope. (`--agent claude` keeps this focused on one target; the dual-home
/// default + shared `--dest` collapse is covered separately below.)
#[test]
fn skill_install_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--agent",
            "claude",
            "--json",
            "--dest",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let installed = v["data"]["installed"].as_array().unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0]["name"], "shipshape-release");
    assert_eq!(installed[0]["agent"], "claude");
    assert_eq!(
        v["warnings"],
        serde_json::json!([]),
        "clean install, no drift"
    );

    let path = dir.path().join("shipshape-release/SKILL.md");
    assert!(path.exists(), "SKILL.md written to disk");
}

/// Install with no name installs the whole catalog.
#[test]
fn skill_install_all_when_no_name() {
    let dir = tempfile::tempdir().unwrap();
    let out = shipshape()
        .args(["skill", "install", "--json", "--dest"])
        .arg(dir.path())
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let installed = v["data"]["installed"].as_array().unwrap();
    assert!(installed.len() >= 2, "the whole catalog is installed: {v}");
}

/// Re-installing the same version is idempotent (no drift warning, exit 0).
#[test]
fn skill_install_reinstall_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..2 {
        let out = shipshape()
            .args(["skill", "install", "shipshape-release", "--json", "--dest"])
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(out.status.success());
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["warnings"], serde_json::json!([]));
    }
}

/// An on-disk skill newer than the binary is refused without `--force` (§17),
/// then overwritten (with a warning) when `--force` is passed.
#[test]
fn skill_install_refuses_newer_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shipshape-release/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "---\nname: shipshape-release\ncli_version: \"9.9.9\"\nschema_version: 1\n---\nstale\n",
    )
    .unwrap();

    // Refused (exit 1, structured error), disk untouched.
    let out = shipshape()
        .args(["skill", "install", "shipshape-release", "--dest"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "skill_version_mismatch");
    assert!(
        std::fs::read_to_string(&path).unwrap().contains("stale"),
        "disk untouched on refusal"
    );

    // --force overwrites and warns.
    let out = shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--json",
            "--force",
            "--dest",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        !v["warnings"].as_array().unwrap().is_empty(),
        "force-downgrade emits a warning"
    );
    assert!(!std::fs::read_to_string(&path).unwrap().contains("stale"));
}

/// `--dest` with `--agent all` writes both runtime shapes under the one dir —
/// they do not collide (`<dest>/<name>/SKILL.md` vs `<dest>/<name>.md`).
#[test]
fn skill_install_dest_with_all_writes_both_shapes() {
    let dir = tempfile::tempdir().unwrap();
    shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--agent",
            "all",
            "--dest",
        ])
        .arg(dir.path())
        .assert()
        .success();
    assert!(
        dir.path().join("shipshape-release/SKILL.md").exists(),
        "claude shape written"
    );
    assert!(
        dir.path().join("shipshape-release.md").exists(),
        "codex shape written"
    );
}

/// A partial-install guard: when any target in the plan is refused, nothing is
/// written (preflight fails the whole command).
#[test]
fn skill_install_preflight_is_all_or_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // Pre-place a newer catalog member (`shipshape-readiness`) so the whole-catalog
    // batch refuses at preflight — after earlier members would otherwise have been
    // written, which is exactly the partial-install we guard.
    let poison = dir.path().join("shipshape-readiness/SKILL.md");
    std::fs::create_dir_all(poison.parent().unwrap()).unwrap();
    std::fs::write(&poison, "---\ncli_version: \"9.9.9\"\n---\n").unwrap();

    let out = shipshape()
        .args(["skill", "install", "--dest"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "batch refused");
    // No earlier member may have been written before the refusal (preflight
    // classifies the whole plan up front, so nothing lands).
    for earlier in ["shipshape-init", "shipshape-release"] {
        assert!(
            !dir.path().join(earlier).join("SKILL.md").exists(),
            "no partial install: preflight refuses before any write ({earlier})"
        );
    }
}

/// Codex installs to the flat `<name>.md` prompt shape (§15 `--agent`).
#[test]
fn skill_install_codex_shape() {
    let dir = tempfile::tempdir().unwrap();
    shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--agent",
            "codex",
            "--dest",
        ])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("codex"));
    assert!(
        dir.path().join("shipshape-release.md").exists(),
        "codex uses a flat <name>.md prompt file"
    );
}

// ── dual-home (Claude Code + pi.dev) ──────────────────────────────────────────

/// With no `--agent`, install **dual-homes**: the same `SKILL.md` lands under
/// both `~/.claude/skills/<name>/` (Claude Code) and `~/.pi/agent/skills/<name>/`
/// (pi.dev), rooted at `$HOME`, and the envelope reports both targets.
#[test]
fn skill_install_default_dual_homes_claude_and_pi() {
    let home = tempfile::tempdir().unwrap();
    let out = shipshape()
        .args(["skill", "install", "shipshape-release", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "dual-home install succeeds");

    let claude = home
        .path()
        .join(".claude/skills/shipshape-release/SKILL.md");
    let pi = home
        .path()
        .join(".pi/agent/skills/shipshape-release/SKILL.md");
    assert!(claude.exists(), "Claude Code target written");
    assert!(pi.exists(), "pi.dev target written");
    // Byte-identical payload in both homes.
    assert_eq!(
        std::fs::read(&claude).unwrap(),
        std::fs::read(&pi).unwrap(),
        "same rendered SKILL.md in both homes"
    );

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let installed = v["data"]["installed"].as_array().unwrap();
    let agents: Vec<&str> = installed
        .iter()
        .map(|e| e["agent"].as_str().unwrap())
        .collect();
    assert!(agents.contains(&"claude"), "claude target reported: {v}");
    assert!(agents.contains(&"pi"), "pi target reported: {v}");
}

/// `--agent claude` writes **only** the Claude Code home — pi.dev is untouched.
#[test]
fn skill_install_agent_claude_only() {
    let home = tempfile::tempdir().unwrap();
    shipshape()
        .args(["skill", "install", "shipshape-release", "--agent", "claude"])
        .env("HOME", home.path())
        .assert()
        .success();
    assert!(
        home.path()
            .join(".claude/skills/shipshape-release/SKILL.md")
            .exists(),
        "claude target written"
    );
    assert!(
        !home
            .path()
            .join(".pi/agent/skills/shipshape-release")
            .exists(),
        "pi.dev target NOT written for --agent claude"
    );
}

/// `--agent pi` writes **only** the pi.dev home — Claude Code is untouched — and
/// the envelope reports exactly one target, labeled `pi`.
#[test]
fn skill_install_agent_pi_only() {
    let home = tempfile::tempdir().unwrap();
    let out = shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--agent",
            "pi",
            "--json",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let installed = v["data"]["installed"].as_array().unwrap();
    assert_eq!(installed.len(), 1, "exactly one target: {v}");
    assert_eq!(installed[0]["agent"], "pi");
    assert!(
        home.path()
            .join(".pi/agent/skills/shipshape-release/SKILL.md")
            .exists(),
        "pi.dev target written"
    );
    assert!(
        !home
            .path()
            .join(".claude/skills/shipshape-release")
            .exists(),
        "claude target NOT written for --agent pi"
    );
}

/// The dual-home install is idempotent: a second run re-writes byte-identical
/// content into both homes with no drift warning and exit 0.
#[test]
fn skill_install_dual_home_is_idempotent() {
    let home = tempfile::tempdir().unwrap();
    let claude = home
        .path()
        .join(".claude/skills/shipshape-release/SKILL.md");
    let pi = home
        .path()
        .join(".pi/agent/skills/shipshape-release/SKILL.md");
    let mut first: Option<Vec<u8>> = None;
    for _ in 0..2 {
        let out = shipshape()
            .args(["skill", "install", "shipshape-release", "--json"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(out.status.success());
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            v["warnings"],
            serde_json::json!([]),
            "no drift on re-install"
        );
        // Both homes present and byte-identical to each other and across runs.
        let cbytes = std::fs::read(&claude).unwrap();
        let pbytes = std::fs::read(&pi).unwrap();
        assert_eq!(cbytes, pbytes, "same bytes in both homes");
        match &first {
            None => first = Some(cbytes),
            Some(prev) => assert_eq!(prev, &cbytes, "stable across re-install"),
        }
    }
}

/// Vendored/bundled filtering: only `SKILL.md` is mirrored into the pi.dev home —
/// the per-skill directory holds exactly that one file, no stray assets.
#[test]
fn skill_install_pi_mirrors_only_skill_md() {
    let home = tempfile::tempdir().unwrap();
    shipshape()
        .args(["skill", "install", "shipshape-release", "--agent", "pi"])
        .env("HOME", home.path())
        .assert()
        .success();
    let skill_dir = home.path().join(".pi/agent/skills/shipshape-release");
    let entries: Vec<String> = std::fs::read_dir(&skill_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["SKILL.md".to_string()],
        "only SKILL.md mirrored"
    );
}

/// A `--dest` shared root makes the shape-sharing Claude + pi.dev targets resolve
/// to one file. The physical *write* collapses (one file on disk), but the
/// *reporting* does not: `--agent all` still yields a `claude` **and** a `pi` row,
/// so a consumer that greps `agent == "pi"` isn't silently dropped.
#[test]
fn skill_install_dest_collapses_shape_sharing_runtimes() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shipshape-release").join("SKILL.md");
    let out = shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--agent",
            "all",
            "--json",
            "--dest",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let installed = v["data"]["installed"].as_array().unwrap();

    // Both shape-sharing runtimes are reported at the one shared path…
    let shared_str = shared.to_string_lossy();
    let at_shared: Vec<&str> = installed
        .iter()
        .filter(|e| e["dest_path"].as_str().unwrap() == shared_str)
        .map(|e| e["agent"].as_str().unwrap())
        .collect();
    assert_eq!(
        at_shared,
        vec!["claude", "pi"],
        "both logical targets reported at the shared file: {v}"
    );
    // …but the file was written only once (it exists; a single physical target).
    assert!(shared.exists(), "shared SKILL.md written");
    // Codex's flat shape is distinct and still present.
    assert!(
        dir.path().join("shipshape-release.md").exists(),
        "codex shape written"
    );
}

/// The final-component overwrite is symlink-safe: installing over a path that is a
/// (dangling) symlink replaces the *link* with a regular file rather than
/// following it. The atomic `rename` guarantees this on POSIX.
#[cfg(unix)]
#[test]
fn skill_install_replaces_final_component_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("shipshape-release");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let target = skill_dir.join("SKILL.md");
    // A dangling symlink at the final component: install must replace the link,
    // not resolve it (which would write through to a nonexistent outside path).
    std::os::unix::fs::symlink(dir.path().join("nowhere-outside"), &target).unwrap();

    shipshape()
        .args([
            "skill",
            "install",
            "shipshape-release",
            "--agent",
            "claude",
            "--dest",
        ])
        .arg(dir.path())
        .assert()
        .success();

    let meta = std::fs::symlink_metadata(&target).unwrap();
    assert!(
        meta.file_type().is_file(),
        "symlink replaced by a regular file, not followed"
    );
    assert!(
        !dir.path().join("nowhere-outside").exists(),
        "the symlink target was never written through"
    );
    assert!(std::fs::read_to_string(&target)
        .unwrap()
        .contains("cli_version"));
}
