//! Explicit Apple builds on GitHub-hosted macOS runners. Nothing here runs unless the caller
//! chooses it; `engine::build` stays on this computer.
//!
//! A build records the working tree as a commit, pushes it to a temporary branch of the
//! `origin` repository, starts the repository's workflow on that branch, waits for the run,
//! then imports the package the run uploaded. The branch is deleted afterwards and the run is
//! cancelled if the build stops early.
use crate::{
    Cancellation, Event, Request,
    engine::{self, Artifact},
    process,
    project::Project,
    tools,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The workflow that [`init`] writes and [`build`] starts.
pub const WORKFLOW: &str = ".github/workflows/cranpose-build.yml";
const WORKFLOW_FILE: &str = "cranpose-build.yml";
const WORKFLOW_TEMPLATE: &str = include_str!("../templates/cranpose-build.yml");
const POLL: Duration = Duration::from_secs(10);
const RUN_TIMEOUT: Duration = Duration::from_secs(3 * 3600);

/// Write the repository workflow that `build` starts. An existing file stays unchanged.
pub fn init(manifest: &Path, cancel: &Cancellation) -> Result<PathBuf> {
    let directory = manifest
        .canonicalize()
        .context("Find Cargo.toml")?
        .parent()
        .context("Manifest directory")?
        .to_owned();
    let root = git_root(&tools::tool("git")?, &directory, cancel)?;
    let path = root.join(WORKFLOW);
    let text = workflow_text();
    match fs::read_to_string(&path) {
        Ok(existing) if existing == text => return Ok(path),
        Ok(_) => bail!(
            "{} exists with other content. Remove it to write the workflow of cranpose-build {}.",
            path.display(),
            env!("CARGO_PKG_VERSION")
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    fs::create_dir_all(path.parent().context("Workflow directory")?)?;
    fs::write(&path, text)?;
    Ok(path)
}

/// The workflow text for this version of cranpose-build.
pub fn workflow_text() -> String {
    WORKFLOW_TEMPLATE.replace("{version}", env!("CARGO_PKG_VERSION"))
}

/// Build an Apple platform on a GitHub macOS runner and import its package.
pub fn build(
    request: &Request,
    cancel: &Cancellation,
    mut emit: impl FnMut(Event),
) -> Result<Artifact> {
    let start = Instant::now();
    ensure!(
        matches!(request.platform.os(), "macos" | "ios"),
        "--on github builds Apple platforms; build {} on this computer",
        request.platform.name()
    );
    emit(Event::Stage {
        message: "Inspecting project".into(),
    });
    let project = Project::load(request, cancel)?;
    let git = tools::tool("git")?;
    let gh = tools::tool("gh")?;
    let directory = project
        .package
        .manifest_path
        .parent()
        .context("Manifest directory")?
        .as_std_path()
        .to_owned();
    let root = git_root(&git, &directory, cancel)?;
    ensure!(
        root.join(WORKFLOW).is_file(),
        "Run cranpose-build init-ci, then commit {WORKFLOW} to the default branch and push it"
    );
    let origin = process::capture(
        git_in(&git, &directory, &["config", "--get", "remote.origin.url"]),
        cancel,
    )
    .context("The repository needs an origin remote on GitHub")?;
    let repository = repository(&origin)?;
    let prefix = process::capture(
        git_in(&git, &directory, &["rev-parse", "--show-prefix"]),
        cancel,
    )?;
    let manifest = format!(
        "{}{}",
        prefix.trim(),
        project
            .package
            .manifest_path
            .file_name()
            .context("Manifest filename")?
    );

    emit(Event::Stage {
        message: "Recording the working tree".into(),
    });
    let commit = snapshot(&git, &directory, cancel, &mut emit)?;
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut remote = Remote {
        git: &git,
        gh: &gh,
        directory: &directory,
        repository: &repository,
        branch: format!("cranpose-build/{}-{millis}", request.platform.name()),
        run: None,
        finished: false,
        pushed: false,
    };
    let mut push = git_in(
        &git,
        &directory,
        &["push", "--no-verify", "--quiet", "origin"],
    );
    push.arg(format!("{commit}:refs/heads/{}", remote.branch))
        .env("GIT_TERMINAL_PROMPT", "0");
    remote.pushed = true;
    process::execute(push, Duration::from_secs(1800), cancel, |text| {
        emit(Event::Log { text: text.into() })
    })?;

    emit(Event::Stage {
        message: format!("Starting {WORKFLOW_FILE} in {repository}"),
    });
    let mut dispatch = Command::new(&gh);
    dispatch
        .args(["workflow", "run", WORKFLOW_FILE, "-R", &repository, "--ref"])
        .arg(&remote.branch)
        .arg("-f")
        .arg(format!("platform={}", request.platform.name()))
        .arg("-f")
        .arg(format!("profile={}", request.profile()))
        .arg("-f")
        .arg(format!("manifest-path={manifest}"))
        .arg("-f")
        .arg(format!("package={}", project.package.name));
    process::capture(dispatch, cancel).with_context(|| {
        format!("GitHub did not start {WORKFLOW}. Commit it to the default branch and push it.")
    })?;
    let run = find_run(&remote, cancel)?;
    remote.run = Some(run.database_id);
    emit(Event::Stage {
        message: format!(
            "Building {} on GitHub: {}",
            request.platform.name(),
            run.url
        ),
    });
    let conclusion = wait(&remote, run.database_id, cancel, &mut emit)?;
    remote.finished = true;
    if conclusion != "success" {
        let mut log = Command::new(&gh);
        log.args([
            "run",
            "view",
            &run.database_id.to_string(),
            "-R",
            &repository,
            "--log-failed",
        ]);
        if let Ok(text) = process::capture(log, cancel) {
            let lines: Vec<_> = text.lines().collect();
            for line in &lines[lines.len().saturating_sub(80)..] {
                emit(Event::Log {
                    text: (*line).into(),
                });
            }
        }
        bail!("The GitHub run ended with {conclusion}: {}", run.url);
    }

    emit(Event::Stage {
        message: "Downloading the package".into(),
    });
    let output = engine::output_directory(&project, request);
    fs::create_dir_all(&output)?;
    let staging = tempfile::Builder::new()
        .prefix("build-")
        .tempdir_in(&output)?;
    let mut download = Command::new(&gh);
    download
        .args([
            "run",
            "download",
            &run.database_id.to_string(),
            "-R",
            &repository,
            "-n",
        ])
        .arg(format!(
            "cranpose-{}-{}",
            request.platform.name(),
            request.profile()
        ))
        .arg("-D")
        .arg(staging.path());
    process::execute(download, Duration::from_secs(1800), cancel, |text| {
        emit(Event::Log { text: text.into() })
    })?;
    let artifact = import(staging.path(), request)?;
    let _ = staging.keep();
    drop(remote);
    emit(Event::Artifact {
        artifact: artifact.clone(),
    });
    emit(Event::Complete {
        elapsed_ms: start.elapsed().as_millis(),
    });
    Ok(artifact)
}

/// Check a downloaded package against its manifest and unpack its application.
fn import(directory: &Path, request: &Request) -> Result<Artifact> {
    let artifact = Artifact::load(&directory.join("artifact.json"))
        .context("The GitHub run uploaded no artifact.json")?;
    ensure!(
        artifact.platform == request.platform,
        "The GitHub run built {}, not {}",
        artifact.platform.name(),
        request.platform.name()
    );
    ensure!(
        engine::hash(&artifact.archive)? == artifact.sha256,
        "The downloaded package does not match its SHA256"
    );
    zip::ZipArchive::new(fs::File::open(&artifact.archive)?)?.extract(directory)?;
    ensure!(
        artifact.path.exists(),
        "The package has no {}",
        artifact.path.display()
    );
    Ok(artifact)
}

/// Commit the working tree, including untracked files that Git does not ignore, without
/// touching the user's index, branches or files.
fn snapshot(
    git: &Path,
    directory: &Path,
    cancel: &Cancellation,
    emit: &mut impl FnMut(Event),
) -> Result<String> {
    let head = process::capture(
        git_in(git, directory, &["rev-parse", "--verify", "HEAD"]),
        cancel,
    )
    .context("Commit once before building on GitHub")?;
    let scratch = tempfile::tempdir()?;
    let index = scratch.path().join("index");
    let current = process::capture(
        git_in(
            git,
            directory,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        ),
        cancel,
    )?;
    let current = PathBuf::from(current.trim());
    // A copy of the real index keeps Git's file stat cache, so only changed files are hashed.
    if current.is_file() {
        fs::copy(&current, &index)?;
    }
    let untracked = process::capture(
        git_in(
            git,
            directory,
            &["ls-files", "--others", "--exclude-standard", ":/"],
        ),
        cancel,
    )?;
    let count = untracked.lines().count();
    if count > 0 {
        emit(Event::Log {
            text: format!("Including {count} untracked files that Git does not ignore"),
        });
    }
    let mut add = git_in(git, directory, &["add", "-A", ":/"]);
    add.env("GIT_INDEX_FILE", &index);
    process::capture(add, cancel)?;
    let mut tree = git_in(git, directory, &["write-tree"]);
    tree.env("GIT_INDEX_FILE", &index);
    let tree = process::capture(tree, cancel)?;
    let mut commit = git_in(
        git,
        directory,
        &["commit-tree", tree.trim(), "-p", head.trim(), "-m"],
    );
    commit.arg("cranpose-build snapshot [skip ci]");
    for (key, value) in [
        ("GIT_AUTHOR_NAME", "cranpose-build"),
        (
            "GIT_AUTHOR_EMAIL",
            "cranpose-build@users.noreply.github.com",
        ),
        ("GIT_COMMITTER_NAME", "cranpose-build"),
        (
            "GIT_COMMITTER_EMAIL",
            "cranpose-build@users.noreply.github.com",
        ),
    ] {
        commit.env(key, value);
    }
    Ok(process::capture(commit, cancel)?.trim().to_owned())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Run {
    database_id: u64,
    url: String,
}
#[derive(Deserialize)]
struct RunState {
    status: String,
    conclusion: String,
}

fn find_run(remote: &Remote<'_>, cancel: &Cancellation) -> Result<Run> {
    let started = Instant::now();
    loop {
        let mut list = Command::new(remote.gh);
        list.args([
            "run",
            "list",
            "-R",
            remote.repository,
            "--workflow",
            WORKFLOW_FILE,
            "--event",
            "workflow_dispatch",
            "--branch",
            &remote.branch,
            "--limit",
            "1",
            "--json",
            "databaseId,url",
        ]);
        let runs: Vec<Run> = serde_json::from_str(&process::capture(list, cancel)?)?;
        if let Some(run) = runs.into_iter().next() {
            return Ok(run);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(120),
            "GitHub did not start {WORKFLOW_FILE} on {}",
            remote.branch
        );
        sleep(Duration::from_secs(2), cancel)?;
    }
}

fn wait(
    remote: &Remote<'_>,
    run: u64,
    cancel: &Cancellation,
    emit: &mut impl FnMut(Event),
) -> Result<String> {
    let started = Instant::now();
    let mut reported = String::new();
    loop {
        let mut view = Command::new(remote.gh);
        view.args([
            "run",
            "view",
            &run.to_string(),
            "-R",
            remote.repository,
            "--json",
            "status,conclusion",
        ]);
        let state: RunState = serde_json::from_str(&process::capture(view, cancel)?)?;
        if state.status == "completed" {
            return Ok(state.conclusion);
        }
        if state.status != reported {
            emit(Event::Log {
                text: format!("GitHub run {}", state.status.replace('_', " ")),
            });
            reported = state.status;
        }
        ensure!(
            started.elapsed() < RUN_TIMEOUT,
            "The GitHub run did not finish in {} hours",
            RUN_TIMEOUT.as_secs() / 3600
        );
        sleep(POLL, cancel)?;
    }
}

/// The pushed branch and the started run. Dropping it cancels an unfinished run and deletes
/// the branch, also after an error or a stop request.
struct Remote<'a> {
    git: &'a Path,
    gh: &'a Path,
    directory: &'a Path,
    repository: &'a str,
    branch: String,
    run: Option<u64>,
    finished: bool,
    pushed: bool,
}
impl Drop for Remote<'_> {
    fn drop(&mut self) {
        if let (Some(run), false) = (self.run, self.finished) {
            let mut cancel = Command::new(self.gh);
            cancel.args(["run", "cancel", &run.to_string(), "-R", self.repository]);
            let _ = quiet(cancel);
        }
        if self.pushed {
            let mut delete = git_in(
                self.git,
                self.directory,
                &["push", "--no-verify", "--quiet", "origin", "--delete"],
            );
            delete.arg(&self.branch);
            let _ = quiet(delete);
        }
    }
}
fn quiet(mut command: Command) -> std::io::Result<ExitStatus> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
}

fn sleep(duration: Duration, cancel: &Cancellation) -> Result<()> {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        cancel.check()?;
        std::thread::sleep(Duration::from_millis(200));
    }
    cancel.check()?;
    Ok(())
}

fn git_in(git: &Path, directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(git);
    command.arg("-C").arg(directory).args(args);
    command
}
fn git_root(git: &Path, directory: &Path, cancel: &Cancellation) -> Result<PathBuf> {
    let root = process::capture(
        git_in(git, directory, &["rev-parse", "--show-toplevel"]),
        cancel,
    )
    .context("--on github needs the project in a Git repository")?;
    Ok(PathBuf::from(root.trim()))
}

/// `HOST/OWNER/REPO` for a GitHub remote URL in HTTPS, SSH or scp form.
pub fn repository(url: &str) -> Result<String> {
    let url = url.trim();
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => rest
            .rsplit_once('@')
            .map_or(rest, |(_, r)| r)
            .split_once('/'),
        None => url.rsplit_once('@').map_or(url, |(_, r)| r).split_once(':'),
    }
    .with_context(|| format!("Origin {url} is not a GitHub repository URL"))?;
    let host = host.split(':').next().unwrap_or(host);
    let host = if host == "ssh.github.com" {
        "github.com"
    } else {
        host
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<_> = path.split('/').collect();
    ensure!(
        !host.is_empty()
            && parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))),
        "Origin {url} is not a GitHub repository URL"
    );
    Ok(format!("{host}/{path}"))
}
