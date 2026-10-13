//! iOS device installs through xtool, driven through the CLI with a scripted `xtool`.
#![cfg(unix)]
use anyhow::{Result, ensure};
use cranpose_build::{Platform, engine};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

const FAKE_XTOOL: &str = "#!/bin/sh\necho \"$*\" >> \"$STATE/calls\"\n";

fn cli(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![root.join("bin")];
    paths.extend(std::env::split_paths(&path));
    Ok(Command::new(env!("CARGO_BIN_EXE_cranpose-build"))
        .args(args)
        .env("PATH", std::env::join_paths(paths)?)
        .env("CRANPOSE_BUILD_HOME", root.join("home"))
        .env("STATE", root)
        .output()?)
}

#[test]
fn an_ipa_that_needs_signing_installs_through_xtool() -> Result<()> {
    let root = tempfile::Builder::new()
        .prefix("cranpose xtool ")
        .tempdir()?;
    let root = root.path();
    fs::create_dir(root.join("bin"))?;
    fs::write(root.join("bin/xtool"), FAKE_XTOOL)?;
    fs::set_permissions(root.join("bin/xtool"), fs::Permissions::from_mode(0o755))?;
    let package = root.join("package");
    fs::create_dir_all(package.join("Payload/Application.app"))?;
    fs::write(package.join("Payload/Application.app/fixture"), "binary")?;
    let artifact = engine::Artifact {
        platform: Platform::Ios,
        path: "Payload/Application.app".into(),
        executable: Some("Payload/Application.app/fixture".into()),
        identifier: "dev.cranpose.fixture".into(),
        sha256: String::new(),
        archive: "application.ipa".into(),
        needs_device_signing: true,
    };
    fs::write(
        package.join("artifact.json"),
        serde_json::to_vec(&artifact)?,
    )?;

    let manifest = package.join("artifact.json");
    let manifest = manifest.to_string_lossy();
    let output = cli(
        root,
        &["--json", "launch", &manifest, "--device", "00008110-TEST"],
    )?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(root.join("calls"))?;
    let app = package.canonicalize()?.join("Payload/Application.app");
    assert_eq!(
        calls,
        format!("install --udid 00008110-TEST {}\n", app.display())
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Application installed"), "{stdout}");

    // Without a device the command asks for one instead of guessing.
    let output = cli(root, &["launch", &manifest])?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--device"));

    if cfg!(target_os = "linux") {
        let output = cli(root, &["devices", "--platform", "ios"])?;
        ensure!(output.status.success());
        assert!(fs::read_to_string(root.join("calls"))?.ends_with("devices --no-wait\n"));
    }
    Ok(())
}
