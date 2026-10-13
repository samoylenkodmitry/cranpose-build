//! A release tag of this repository serves its reusable workflow, which installs the binary of
//! that same release.

#[test]
fn workflows_pin_this_release() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let reusable = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/apple.yml"),
    )?;
    assert!(
        reusable.contains(&format!("default: '{version}'")),
        "apple.yml must install cranpose-build {version} by default"
    );
    assert!(cranpose_build::github::workflow_text().contains(&format!("apple.yml@v{version}")));
    Ok(())
}
