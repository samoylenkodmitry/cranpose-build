use anyhow::Result;
use cranpose_build::{Cancellation, Config, Host, Platform, Request, engine};
use std::{fs, process::Command};

fn fixture() -> Result<tempfile::TempDir> {
    let root = tempfile::Builder::new()
        .prefix("cranpose build spaces ")
        .tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='1.2.3'\nedition='2024'\n",
    )?;
    fs::write(
        root.path().join("src/main.rs"),
        "fn main(){println!(\"fixture launched\");}\n",
    )?;
    let status = Command::new(cranpose_build::tools::tool("cargo")?)
        .args(["generate-lockfile", "--offline"])
        .current_dir(root.path())
        .status()?;
    assert!(status.success());
    Ok(root)
}

#[test]
fn build_package_and_launch_from_paths_with_spaces() -> Result<()> {
    let root = fixture()?;
    fs::create_dir(root.path().join("assets"))?;
    fs::write(root.path().join("assets/message.txt"), "hello")?;
    fs::write(
        root.path().join("CranposeBuild.toml"),
        "[app]\nname='Fixture & Test'\nidentifier='dev.cranpose.fixture'\nassets=['assets']\n",
    )?;
    let request = Request::new(root.path().join("Cargo.toml"), Host::default().native());
    let cancel = Cancellation::default();
    let plan = engine::plan(&request, &cancel)?;
    assert_eq!(plan.backend, "cargo");
    assert!(plan.arguments.contains(&"--locked".into()));
    let artifact = engine::build(&request, &cancel, |_| {})?;
    assert!(artifact.path.exists());
    assert_eq!(artifact.sha256, engine::hash(&artifact.archive)?);
    let mut zip = zip::ZipArchive::new(fs::File::open(&artifact.archive)?)?;
    assert!(zip.file_names().any(|n| n.ends_with("assets/message.txt")));
    let metadata: engine::Artifact = serde_json::from_slice(&fs::read(
        artifact
            .archive
            .parent()
            .expect("artifact directory")
            .join("artifact.json"),
    )?)?;
    assert_eq!(metadata.sha256, artifact.sha256);
    assert!(!metadata.path.is_absolute());
    let resolved = engine::Artifact::load(
        &artifact
            .archive
            .parent()
            .expect("artifact directory")
            .join("artifact.json"),
    )?;
    assert_eq!(resolved.path.canonicalize()?, artifact.path.canonicalize()?);
    if cfg!(target_os = "macos") {
        let plist = plist::Value::from_file(artifact.path.join("Contents/Info.plist"))?;
        assert_eq!(
            plist.as_dictionary().expect("dictionary")["CFBundleName"].as_string(),
            Some("Fixture & Test")
        );
    }
    assert!(zip.by_index(0)?.size() > 0);
    drop(zip);
    let moved = root.path().join("moved package");
    fs::rename(
        artifact.archive.parent().expect("package directory"),
        &moved,
    )?;
    let relocated = engine::Artifact::load(&moved.join("artifact.json"))?;
    assert_eq!(relocated.sha256, engine::hash(&relocated.archive)?);
    let mut output = String::new();
    engine::run(&relocated, None, &cancel, |e| {
        if let engine::Event::Log { text } = e {
            output.push_str(&text);
        }
    })?;
    assert!(output.contains("fixture launched"));
    let mut invalid = serde_json::to_value(&metadata)?;
    invalid["executable"] = serde_json::json!("../outside");
    fs::write(moved.join("invalid.json"), serde_json::to_vec(&invalid)?)?;
    assert!(engine::Artifact::load(&moved.join("invalid.json")).is_err());
    Ok(())
}

#[test]
fn planning_is_read_only_and_selects_cross_backends() -> Result<()> {
    let root = fixture()?;
    let source = fs::read(root.path().join("Cargo.toml"))?;
    let mut request = Request::new(root.path().join("Cargo.toml"), Platform::Windows);
    request.release = true;
    request.offline = true;
    let plan = engine::plan(&request, &Cancellation::default())?;
    assert_eq!(
        plan.backend,
        if cfg!(windows) { "cargo" } else { "cargo-xwin" }
    );
    assert!(plan.arguments.contains(&"--release".into()));
    assert!(plan.arguments.contains(&"--offline".into()));
    assert!(
        plan.expected_binary
            .expect("executable")
            .ends_with("x86_64-pc-windows-msvc/release/fixture.exe")
    );
    assert_eq!(source, fs::read(root.path().join("Cargo.toml"))?);
    assert!(!root.path().join("dist").exists());
    Ok(())
}

#[test]
fn rejects_unknown_config_and_missing_targets() -> Result<()> {
    let root = fixture()?;
    fs::write(root.path().join("CranposeBuild.toml"), "mispelled=true")?;
    assert!(Config::read(root.path()).is_err());
    fs::write(
        root.path().join("CranposeBuild.toml"),
        "[targets.typo]\nbin='fixture'",
    )?;
    assert!(Config::read(root.path()).is_err());
    fs::write(
        root.path().join("CranposeBuild.toml"),
        format!(
            "[targets.{}]\nbin='nonexistent'",
            Host::default().native().name()
        ),
    )?;
    let request = Request::new(root.path().join("Cargo.toml"), Host::default().native());
    assert!(engine::plan(&request, &Cancellation::default()).is_err());
    Ok(())
}

#[test]
fn failed_compilation_produces_no_successful_artifact() -> Result<()> {
    let root = fixture()?;
    fs::write(
        root.path().join("src/main.rs"),
        "compile_error!(\"expected fixture error\");",
    )?;
    let request = Request::new(root.path().join("Cargo.toml"), Host::default().native());
    let result = engine::build(&request, &Cancellation::default(), |_| {});
    assert!(result.is_err());
    assert!(!root.path().join("dist").exists());
    Ok(())
}

#[test]
fn platform_matrix_has_no_implicit_remote_routes() {
    for os in ["macos", "linux", "windows"] {
        let host = Host {
            os: os.into(),
            arch: "x86_64".into(),
        };
        for target in Platform::ALL {
            assert!(
                ["cargo", "cargo-xwin", "cargo-zigbuild", "gradle"]
                    .contains(&target.backend(&host))
            );
        }
        assert_eq!(host.native().os(), os);
    }
}
