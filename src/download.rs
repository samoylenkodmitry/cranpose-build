//! SDK downloads are HTTPS-only, size-bounded and checksum-verified before use.
use crate::{Cancellation, Host, engine, process, tools};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const ZIG_VERSION: &str = "0.15.2";
pub fn zig_directory() -> PathBuf {
    tools::managed_root()
        .join("tools")
        .join(format!("zig-{ZIG_VERSION}"))
}
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(300)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .build()
        .into()
}
pub fn verify(path: &Path, expected: &str) -> Result<()> {
    ensure!(
        expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA256 metadata"
    );
    ensure!(
        engine::hash(path)?.eq_ignore_ascii_case(expected),
        "Download checksum mismatch; no executable was installed"
    );
    Ok(())
}
pub fn install_zig(cancel: &Cancellation, mut log: impl FnMut(&str)) -> Result<()> {
    if tools::find("zig").is_some() {
        return Ok(());
    }
    cancel.check()?;
    let host = Host::default();
    let key = format!("{}-{}", host.arch, host.os);
    log(&format!("Fetching Zig {ZIG_VERSION} for {key}"));
    let http = agent();
    let index: Value = http
        .get("https://ziglang.org/download/index.json")
        .call()?
        .body_mut()
        .read_json()?;
    let entry = &index[ZIG_VERSION][&key];
    let url = entry["tarball"]
        .as_str()
        .context("Zig does not provide this host architecture")?;
    ensure!(
        url.starts_with(&format!("https://ziglang.org/download/{ZIG_VERSION}/")),
        "Unexpected Zig download origin"
    );
    let checksum = entry["shasum"].as_str().context("Missing Zig checksum")?;
    let destination = zig_directory();
    let parent = destination.parent().context("Tool directory")?;
    fs::create_dir_all(parent)?;
    let temp = tempfile::Builder::new()
        .prefix("zig-download-")
        .tempdir_in(parent)?;
    let archive = temp.path().join(if host.os == "windows" {
        "zig.zip"
    } else {
        "zig.tar.xz"
    });
    let mut reader = http.get(url).call()?.into_body().into_reader();
    let mut writer = fs::File::create(&archive)?;
    let mut buffer = [0; 65536];
    let mut count = 0u64;
    loop {
        cancel.check()?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        ensure!(
            count <= 256 * 1024 * 1024,
            "Zig archive exceeds download limit"
        );
        writer.write_all(&buffer[..read])?;
    }
    writer.sync_all()?;
    drop(writer);
    verify(&archive, checksum)?;
    let extracted = temp.path().join("extracted");
    fs::create_dir(&extracted)?;
    if host.os == "windows" {
        zip::ZipArchive::new(fs::File::open(&archive)?)?.extract(&extracted)?;
    } else {
        let mut unpack = Command::new(tools::tool("tar")?);
        unpack.arg("-xf").arg(&archive).arg("-C").arg(&extracted);
        process::execute(unpack, Duration::from_secs(120), cancel, &mut log)?;
    }
    let roots: Vec<_> = fs::read_dir(&extracted)?.collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        roots.len() == 1 && roots[0].file_type()?.is_dir(),
        "Unexpected Zig archive layout"
    );
    let root = roots[0].path();
    let executable = root.join(if host.os == "windows" {
        "zig.exe"
    } else {
        "zig"
    });
    let mut check = Command::new(&executable);
    check.arg("version");
    ensure!(
        process::capture(check, cancel)?.trim() == ZIG_VERSION,
        "Zig version mismatch"
    );
    fs::write(
        root.join("cranpose-download.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"url":url,"sha256":checksum,"version":ZIG_VERSION}),
        )?,
    )?;
    fs::rename(&root, &destination)
        .with_context(|| format!("Publish Zig in {}", destination.display()))?;
    log(&format!(
        "Installed verified Zig {ZIG_VERSION} in {}",
        destination.display()
    ));
    Ok(())
}
