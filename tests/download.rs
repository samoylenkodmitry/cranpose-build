use anyhow::Result;
#[test]
fn checksum_rejects_corruption_and_missing_metadata() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("payload");
    std::fs::write(&path, b"abc")?;
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    cranpose_build::download::verify(&path, expected)?;
    std::fs::write(&path, b"abd")?;
    assert!(cranpose_build::download::verify(&path, expected).is_err());
    assert!(cranpose_build::download::verify(&path, "").is_err());
    Ok(())
}
