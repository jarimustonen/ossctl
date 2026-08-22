use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

pub struct TempRepo {
    dir: TempDir,
    remote: TempDir,
    name: String,
}

impl TempRepo {
    pub fn new(status: &str) -> Self {
        let dir = tempfile::tempdir().expect("create temporary repository");
        let remote = tempfile::tempdir().expect("create temporary bare remote");
        let repo = Self {
            dir,
            remote,
            name: "e2e-fixture".to_string(),
        };
        repo.write_contract(status);
        fs::write(
            repo.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                repo.name
            ),
        )
        .expect("write manifest");
        fs::create_dir_all(repo.path().join("src")).expect("create source directory");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("write source");
        repo.git(&["init"]);
        repo.git(&["config", "user.name", "ossctl e2e"]);
        repo.git(&["config", "user.email", "e2e@example.invalid"]);
        repo.git(&["add", "."]);
        repo.git(&["commit", "-m", "initial fixture"]);
        let output = Command::new("git")
            .args(["init", "--bare"])
            .arg(repo.remote.path())
            .output()
            .expect("initialize bare remote");
        assert!(
            output.status.success(),
            "initialize bare remote: {output:?}"
        );
        repo.git(&[
            "remote",
            "add",
            "origin",
            repo.remote.path().to_str().expect("utf-8 remote path"),
        ]);
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn append_commit(&self, name: &str, contents: &str) {
        fs::write(self.path().join(name), contents).expect("write committed fixture file");
        self.git(&["add", name]);
        self.git(&["commit", "-m", "fixture change"]);
    }

    pub fn use_cargo_dist_target(&self) {
        fs::write(
            self.path().join("OSS-RELEASE.md"),
            "---\nschema_version: 1\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  - {ecosystem: rust, package: e2e-fixture, registry: gh-releases, adapter: cargo-dist}\ndistribution:\n  adapter: cargo-dist\n  gh_releases: true\n  platforms: [aarch64-apple-darwin]\nversioning: semver\nchangelog:\n  mode: curated\nrelease:\n  model: gated\nlicense: MIT\n---\n# e2e-fixture\n",
        )
        .expect("write cargo-dist contract");
        self.git(&["add", "OSS-RELEASE.md"]);
        self.git(&["commit", "-m", "configure cargo-dist fixture"]);
    }

    /// Rewrite the fixture as a **publish-none** repo: an explicit `targets: []`
    /// contract plus the `publish = false` manifest that backs it up — the private,
    /// never-published service shape.
    pub fn use_publish_none_contract(&self) {
        fs::write(
            self.path().join("OSS-RELEASE.md"),
            "---\nschema_version: 2\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\nversioning: semver\nchangelog:\n  mode: curated\nrelease:\n  model: gated\nlicense: MIT\n---\n# e2e-fixture\n",
        )
        .expect("write publish-none contract");
        fs::write(
            self.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\npublish = false\n",
                self.name
            ),
        )
        .expect("write publish = false manifest");
        self.git(&["add", "OSS-RELEASE.md", "Cargo.toml"]);
        self.git(&["commit", "-m", "configure publish-none fixture"]);
    }

    pub fn journal_dir(&self) -> PathBuf {
        self.path().join(".git/ossctl/releases")
    }

    pub fn run(&self, shims: &Shims, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ossctl"));
        command
            .args(args)
            .current_dir(self.path())
            .env("PATH", shims.path_with_system_path())
            .env("SHIM_DIR", shims.dir.path())
            .env("HOME", self.path().join("home"))
            .env("CARGO_HOME", self.path().join("cargo-home"))
            .env("RUSTUP_HOME", self.path().join("rustup-home"))
            .env("OSSCTL_E2E_FAST_CLOCK", "1");
        command.output().expect("run ossctl")
    }

    fn write_contract(&self, status: &str) {
        fs::write(
            self.path().join("OSS-RELEASE.md"),
            format!(
                "---\nschema_version: 1\nstatus: {status}\nmaturity: mvp\necosystems: [rust]\ntargets:\n  - {{ecosystem: rust, package: e2e-fixture, registry: crates.io, adapter: cargo-publish}}\nversioning: semver\nchangelog:\n  mode: curated\nrelease:\n  model: gated\nlicense: MIT\n---\n# e2e-fixture\n"
            ),
        )
        .expect("write contract");
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

pub struct Shims {
    dir: TempDir,
}

impl Shims {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("create shim directory");
        let shims = Self { dir };
        fs::create_dir_all(shims.dir.path().join("spec")).expect("create shim spec directory");
        fs::write(shims.dir.path().join("log"), "").expect("create shim log");
        for command in ["cargo", "dist", "gh", "curl", "sha256sum", "shasum"] {
            shims.write_script(command);
            shims.set(command, 0, "");
        }
        shims
    }

    pub fn set(&self, command: &str, exit_code: i32, stdout: &str) {
        fs::write(
            self.dir.path().join("spec").join(format!("{command}.exit")),
            exit_code.to_string(),
        )
        .expect("write shim exit status");
        fs::write(
            self.dir
                .path()
                .join("spec")
                .join(format!("{command}.stdout")),
            stdout,
        )
        .expect("write shim stdout");
    }

    pub fn set_script(&self, command: &str, script: &str) {
        let path = self.dir.path().join(command);
        fs::write(&path, script).expect("write custom shim script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("make custom shim executable");
        }
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.dir.path().join("log")).expect("read shim log")
    }

    pub fn assert_called(&self, command: &str) {
        assert!(
            self.log().lines().any(|line| line.starts_with(command)),
            "expected {command} shim to be called; log was:\n{}",
            self.log()
        );
    }

    pub fn path_with_system_path(&self) -> std::ffi::OsString {
        let mut paths = vec![self.dir.path().to_path_buf()];
        paths.extend(env::split_paths(&env::var_os("PATH").expect("PATH is set")));
        env::join_paths(paths).expect("build controlled PATH")
    }

    fn write_script(&self, command: &str) {
        let script = r#"#!/bin/sh
name=${0##*/}
printf '%s' "$name" >> "$SHIM_DIR/log"
for arg in "$@"; do printf ' <%s>' "$arg" >> "$SHIM_DIR/log"; done
printf '\n' >> "$SHIM_DIR/log"
stdout="$SHIM_DIR/spec/$name.stdout"
exit_code=$(cat "$SHIM_DIR/spec/$name.exit")
cat "$stdout"
exit "$exit_code"
"#;
        let path = self.dir.path().join(command);
        fs::write(&path, script).expect("write shim script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("make shim executable");
        }
    }
}

pub fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON stdout ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

pub fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).unwrap_or_else(|error| {
        panic!(
            "expected JSON stderr ({error}): {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    value["error"]["code"]
        .as_str()
        .expect("error code")
        .to_string()
}

pub fn plan_id(repo: &TempRepo, shims: &Shims) -> String {
    let output = repo.run(shims, &["--json", "release", "plan"]);
    assert!(output.status.success(), "plan failed: {output:?}");
    json(&output)["data"]["plan_id"]
        .as_str()
        .expect("plan id")
        .to_string()
}

pub fn only_run_id(repo: &TempRepo) -> String {
    let mut runs = fs::read_dir(repo.journal_dir())
        .expect("read journal directory")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(std::fs::FileType::is_dir)
                .map(|_| entry)
        })
        .map(|entry| entry.file_name().into_string().expect("utf-8 run id"))
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1, "expected exactly one release run: {runs:?}");
    runs.pop().expect("release run")
}
