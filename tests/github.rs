//! `--on github` through the CLI, against a local bare repository that stands in for GitHub
//! and a scripted `gh` that stands in for the workflow run.
#![cfg(unix)]
use anyhow::{Result, ensure};
use cranpose_build::{Platform, engine};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const FAKE_GH: &str = r#"#!/bin/sh
echo "$*" >> "$STATE/calls"
case "$1 $2" in
  "workflow run")
    branch=""; previous=""
    for argument in "$@"; do
      [ "$previous" = "--ref" ] && branch="$argument"
      previous="$argument"
    done
    echo "$branch" > "$STATE/branch"
    git --git-dir="$REMOTE" show "$branch:app/src/main.rs" > "$STATE/main.rs"
    git --git-dir="$REMOTE" show "$branch:app/src/extra.rs" > /dev/null && echo yes > "$STATE/untracked"
    git --git-dir="$REMOTE" show -s --format=%s "$branch" > "$STATE/subject"
    ;;
  "run list") echo '[{"databaseId":7,"url":"https://github.com/example/app/actions/runs/7"}]' ;;
  "run view")
    case "$*" in
      *--log-failed*) echo "error: the fixture failed to compile" ;;
      *) echo "{\"status\":\"completed\",\"conclusion\":\"$CONCLUSION\"}" ;;
    esac
    ;;
  "run download")
    previous=""
    for argument in "$@"; do
      [ "$previous" = "-D" ] && cp "$PACKAGE"/* "$argument"/
      previous="$argument"
    done
    ;;
esac
"#;

struct Fixture {
    root: tempfile::TempDir,
}
impl Fixture {
    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }
    fn new() -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("cranpose github ")
            .tempdir()?;
        let fixture = Self { root };
        let app = fixture.path("repo/app");
        fs::create_dir_all(app.join("src"))?;
        fs::write(
            app.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='1.2.3'\nedition='2024'\n",
        )?;
        fs::write(app.join("src/main.rs"), "fn main() {}\n")?;
        let status = Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(&app)
            .status()?;
        ensure!(status.success());
        let remote = fixture.path("remote.git");
        git(fixture.root.path(), &["init", "-q", "--bare", "remote.git"])?;
        let repo = fixture.path("repo");
        git(&repo, &["init", "-q", "-b", "main"])?;
        git(&repo, &["config", "user.name", "Fixture"])?;
        git(&repo, &["config", "user.email", "fixture@example.com"])?;
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/app.git",
            ],
        )?;
        git(
            &repo,
            &[
                "config",
                &format!("url.{}.insteadOf", remote.display()),
                "https://github.com/example/app.git",
            ],
        )?;
        let init = cli(&fixture, &["init-ci", "--manifest-path", "app/Cargo.toml"])?;
        ensure!(init.status.success(), "{}", text(&init));
        let workflow = fs::read_to_string(repo.join(".github/workflows/cranpose-build.yml"))?;
        ensure!(workflow.contains(&format!(
            "cranpose-build/.github/workflows/apple.yml@v{}",
            env!("CARGO_PKG_VERSION")
        )));
        let again = cli(&fixture, &["init-ci", "--manifest-path", "app/Cargo.toml"])?;
        ensure!(again.status.success(), "{}", text(&again));
        git(&repo, &["add", "-A"])?;
        git(&repo, &["commit", "-q", "-m", "Start"])?;
        git(&repo, &["push", "-q", "origin", "main"])?;

        // Work the user has not committed: one changed file and one new file.
        fs::write(
            app.join("src/main.rs"),
            "fn main() { println!(\"edited\"); }\n",
        )?;
        fs::write(app.join("src/extra.rs"), "\n")?;

        // The package the scripted run uploads.
        let built = fixture.path("built");
        let bundle = built.join("Application.app/Contents/MacOS");
        fs::create_dir_all(&bundle)?;
        fs::write(bundle.join("fixture"), "binary")?;
        fs::set_permissions(bundle.join("fixture"), fs::Permissions::from_mode(0o755))?;
        let package = fixture.path("package");
        fs::create_dir(&package)?;
        engine::archive_path(
            &built.join("Application.app"),
            &package.join("application.zip"),
        )?;
        let artifact = engine::Artifact {
            platform: Platform::Mac,
            path: "Application.app".into(),
            executable: Some("Application.app/Contents/MacOS/fixture".into()),
            identifier: "dev.cranpose.fixture".into(),
            sha256: engine::hash(&package.join("application.zip"))?,
            archive: "application.zip".into(),
            needs_device_signing: false,
        };
        fs::write(
            package.join("artifact.json"),
            serde_json::to_vec(&artifact)?,
        )?;

        let bin = fixture.path("bin");
        fs::create_dir(&bin)?;
        fs::write(bin.join("gh"), FAKE_GH)?;
        fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755))?;
        fs::create_dir(fixture.path("state"))?;
        Ok(fixture)
    }
    fn build(&self, conclusion: &str) -> Result<Output> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cranpose-build"));
        command
            .args([
                "--json",
                "build",
                "--platform",
                "mac",
                "--on",
                "github",
                "--manifest-path",
                "app/Cargo.toml",
            ])
            .env("CONCLUSION", conclusion);
        Ok(self.environment(&mut command).output()?)
    }
    fn environment<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.path("bin")];
        paths.extend(std::env::split_paths(&path));
        command
            .current_dir(self.path("repo"))
            .env("PATH", std::env::join_paths(paths).unwrap_or_default())
            .env("CRANPOSE_BUILD_HOME", self.path("home"))
            .env("STATE", self.path("state"))
            .env("REMOTE", self.path("remote.git"))
            .env("PACKAGE", self.path("package"))
    }
    fn remote_branches(&self) -> Result<String> {
        git(
            self.root.path(),
            &[
                "--git-dir",
                "remote.git",
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/cranpose-build",
            ],
        )
    }
}

fn cli(fixture: &Fixture, args: &[&str]) -> Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cranpose-build"));
    command.args(args);
    Ok(fixture.environment(&mut command).output()?)
}
fn git(directory: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()?;
    ensure!(output.status.success(), "git {args:?}: {}", text(&output));
    Ok(String::from_utf8(output.stdout)?)
}
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn builds_a_snapshot_of_the_working_tree_and_imports_the_package() -> Result<()> {
    let fixture = Fixture::new()?;
    let output = fixture.build("success")?;
    ensure!(output.status.success(), "{}", text(&output));

    let state = fixture.path("state");
    assert_eq!(
        fs::read_to_string(state.join("main.rs"))?,
        "fn main() { println!(\"edited\"); }\n"
    );
    assert_eq!(fs::read_to_string(state.join("untracked"))?, "yes\n");
    assert!(fs::read_to_string(state.join("subject"))?.contains("[skip ci]"));
    let calls = fs::read_to_string(state.join("calls"))?;
    assert!(calls.contains("-R github.com/example/app"));
    assert!(calls.contains("-f manifest-path=app/Cargo.toml"));
    assert!(calls.contains("-f profile=debug"));
    assert!(calls.contains("-n cranpose-mac-debug"));
    assert!(!calls.contains("run cancel"));

    let artifact = String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["type"] == "artifact")
        .map(|event| event["artifact"].clone());
    let artifact: engine::Artifact =
        serde_json::from_value(artifact.ok_or_else(|| anyhow::anyhow!("no artifact event"))?)?;
    let executable = artifact
        .executable
        .ok_or_else(|| anyhow::anyhow!("no executable"))?;
    assert_eq!(fs::read_to_string(&executable)?, "binary");
    assert_eq!(
        fs::metadata(&executable)?.permissions().mode() & 0o111,
        0o111
    );
    let output_directory = fixture
        .path("repo/app")
        .canonicalize()?
        .join("dist/cranpose/mac/debug");
    assert!(artifact.path.starts_with(output_directory));
    let reloaded = engine::Artifact::load(
        &artifact
            .archive
            .parent()
            .ok_or_else(|| anyhow::anyhow!("no directory"))?
            .join("artifact.json"),
    )?;
    assert_eq!(reloaded.path, artifact.path);

    // The temporary branch is gone, and the user's index and files are untouched.
    assert_eq!(fixture.remote_branches()?, "");
    let status = git(
        &fixture.path("repo"),
        &["status", "--porcelain", "--", "app/src"],
    )?;
    assert_eq!(status, " M app/src/main.rs\n?? app/src/extra.rs\n");
    Ok(())
}

#[test]
fn a_failed_run_reports_its_log_and_removes_the_branch() -> Result<()> {
    let fixture = Fixture::new()?;
    let output = fixture.build("failure")?;
    assert!(!output.status.success());
    let all = text(&output);
    assert!(all.contains("the fixture failed to compile"), "{all}");
    assert!(all.contains("ended with failure"), "{all}");
    assert_eq!(fixture.remote_branches()?, "");
    assert!(!fixture.path("repo/app/dist").exists());
    Ok(())
}

#[test]
fn origin_urls_name_the_github_repository() -> Result<()> {
    for (url, expected) in [
        ("https://github.com/owner/app.git", "github.com/owner/app"),
        ("https://token@github.com/owner/app", "github.com/owner/app"),
        ("git@github.com:owner/app.git", "github.com/owner/app"),
        (
            "ssh://git@ssh.github.com:443/owner/app.git",
            "github.com/owner/app",
        ),
        (
            "https://ghe.example.com/team/app.git/",
            "ghe.example.com/team/app",
        ),
    ] {
        assert_eq!(cranpose_build::github::repository(url)?, expected);
    }
    for url in [
        "/srv/app.git",
        "https://github.com/owner",
        "git@github.com:owner/app/extra",
    ] {
        assert!(cranpose_build::github::repository(url).is_err(), "{url}");
    }
    Ok(())
}
