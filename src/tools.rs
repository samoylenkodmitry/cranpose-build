use crate::{Cancellation, Host, Platform, process};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
pub fn managed_root() -> PathBuf {
    std::env::var_os("CRANPOSE_BUILD_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|p| p.join(".cranpose-build")))
        .unwrap_or_else(|| PathBuf::from(".cranpose-build"))
}
pub fn find(name: &str) -> Option<PathBuf> {
    let mut dirs = vec![managed_root().join("bin"), crate::download::zig_directory()];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if let Some(cargo) = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|p| p.join(".cargo")))
    {
        dirs.push(cargo.join("bin"));
    }
    if let Some(java) = std::env::var_os("JAVA_HOME") {
        dirs.push(PathBuf::from(java).join("bin"));
    }
    if let Some(llvm) = std::env::var_os("LLVM_HOME") {
        dirs.push(PathBuf::from(llvm).join("bin"));
    }
    if cfg!(target_os = "macos") {
        dirs.extend([
            PathBuf::from("/opt/homebrew/opt/llvm/bin"),
            PathBuf::from("/usr/local/opt/llvm/bin"),
        ]);
    }
    if let Some(sdk) = android_sdk() {
        dirs.push(sdk.join("platform-tools"));
    }
    for dir in dirs {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
        if cfg!(windows) {
            let path = path.with_extension("exe");
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
pub fn tool(name: &str) -> Result<PathBuf> {
    find(name).with_context(|| {
        format!("Missing {name}. Run cranpose-build doctor for setup instructions.")
    })
}
pub fn android_sdk() -> Option<PathBuf> {
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(path) = std::env::var_os(key) {
            return Some(path.into());
        }
    }
    let home = home()?;
    [
        home.join("Library/Android/sdk"),
        home.join("Android/Sdk"),
        home.join("AppData/Local/Android/Sdk"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
}
pub fn apply_environment(command: &mut Command) -> Result<()> {
    let mut paths = vec![managed_root().join("bin"), crate::download::zig_directory()];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    for name in ["cargo", "java", "zig", "clang"] {
        if let Some(path) = find(name).and_then(|p| p.parent().map(Path::to_owned))
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    command.env("PATH", std::env::join_paths(paths)?);
    if let Some(sdk) = android_sdk() {
        command.env("ANDROID_HOME", sdk);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ready: bool,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Doctor {
    pub host: Host,
    pub platform: Platform,
    pub checks: Vec<Check>,
    pub ready: bool,
    pub launch: String,
}
pub fn doctor(platform: Platform, cancel: &Cancellation) -> Doctor {
    let host = Host::default();
    let mut checks = Vec::new();
    let mut required = vec!["cargo", "rustc", "rustup"];
    match platform.backend(&host) {
        "cargo-xwin" => required.extend(["cargo-xwin", "clang-cl", "llvm-lib"]),
        "cargo-zigbuild" => required.extend(["cargo-zigbuild", "zig"]),
        "gradle" => required.push("java"),
        _ => {}
    }
    if platform.os() == "ios" || (platform.os() == "macos" && host.os == "macos") {
        required.push("xcrun");
        required.push("codesign");
    }
    for name in required {
        let path = find(name);
        checks.push(Check {
            name: name.into(),
            ready: path.is_some(),
            detail: path
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| installation_hint(name).into()),
        });
    }
    if platform.os() == "ios" && host.os != "macos" {
        checks.push(Check { name: "Xcode host".into(), ready: false, detail: "iOS packaging, signing and simulator launch require macOS with Xcode. Use an explicit Mac builder.".into() });
    }
    if platform.os() == "macos" && host.os != "macos" {
        let sdk = std::env::var_os("SDKROOT").map(PathBuf::from);
        checks.push(Check { name: "Apple SDK".into(), ready: sdk.as_ref().is_some_and(|p| p.is_dir()), detail: "Set SDKROOT to your locally provisioned macOS SDK; the tool does not redistribute Apple SDKs.".into() });
    }
    if platform == Platform::Android {
        checks.push(Check { name: "Android SDK".into(), ready: android_sdk().is_some_and(|p| p.is_dir()), detail: "Install Android SDK, NDK and build tools with Android Studio; review their licenses. ANDROID_HOME selects the SDK.".into() });
    } else if let Some(rustup) = find("rustup") {
        let mut command = Command::new(rustup);
        command.args(["target", "list", "--installed"]);
        let installed = process::capture(command, cancel).unwrap_or_default();
        checks.push(Check {
            name: platform.triple().into(),
            ready: installed.lines().any(|line| line == platform.triple()),
            detail: format!("cranpose-build setup --platform {}", platform.name()),
        });
    }
    let launch = if platform.os() == host.os {
        "Native launch on this host"
    } else if platform == Platform::Android {
        "adb install and launch on an explicit device/emulator"
    } else if platform.simulator() && host.os == "macos" {
        "Install and launch in an explicit booted iOS simulator"
    } else if platform == Platform::Ios && host.os == "macos" {
        "Signed device install and launch with Xcode devicectl"
    } else {
        "Build only on this host; launch on a matching OS or explicit VM/device"
    }
    .into();
    let ready = checks.iter().all(|c| c.ready);
    Doctor {
        host,
        platform,
        checks,
        ready,
        launch,
    }
}
fn installation_hint(name: &str) -> &'static str {
    match name {
        "cargo" | "rustc" | "rustup" => "Install Rust from https://rustup.rs",
        "cargo-xwin" | "cargo-zigbuild" => "Run cranpose-build setup --platform <platform>",
        "zig" => "Install Zig from https://ziglang.org/download/ and add it to PATH",
        "clang" | "llvm-lib" | "lld-link" => {
            "Install LLVM from https://releases.llvm.org and add its bin directory to PATH"
        }
        "java" => "Install a JDK 17 or newer and set JAVA_HOME",
        "xcrun" | "codesign" => "Install Xcode and select it with xcode-select",
        _ => "Install the platform SDK tool and add it to PATH",
    }
}

/// Installs pinned Cargo backends into the tool's private directory. SDK licenses remain explicit.
pub fn setup(platform: Platform, cancel: &Cancellation, mut log: impl FnMut(&str)) -> Result<()> {
    let host = Host::default();
    if platform.os() == "ios" && host.os != "macos" {
        bail!("Set up iOS on a Mac with Xcode");
    }
    let mut target = Command::new(tool("rustup")?);
    target.args(["target", "add", platform.triple()]);
    process::execute(target, Duration::from_secs(900), cancel, &mut log)?;
    let backend = match platform.backend(&host) {
        "cargo-xwin" => Some(("cargo-xwin", "0.23.0")),
        "cargo-zigbuild" => Some(("cargo-zigbuild", "0.23.4")),
        _ => None,
    };
    if let Some((name, version)) = backend {
        let mut install = Command::new(tool("cargo")?);
        install
            .args(["install", "--locked", "--version", version, "--root"])
            .arg(managed_root())
            .arg(name);
        process::execute(install, Duration::from_secs(3600), cancel, &mut log)?;
    }
    if platform.backend(&host) == "cargo-zigbuild" {
        crate::download::install_zig(cancel, &mut log)?;
    }
    Ok(())
}
