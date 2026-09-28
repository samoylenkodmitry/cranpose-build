use crate::{Cancellation, Host, Platform, Request, process, project::Project, tools};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Stage { message: String },
    Log { text: String },
    Artifact { artifact: Artifact },
    Complete { elapsed_ms: u128 },
    Error { message: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub platform: Platform,
    pub path: PathBuf,
    pub executable: Option<PathBuf>,
    pub identifier: String,
    pub sha256: String,
    pub archive: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub platform: Platform,
    pub backend: String,
    pub directory: PathBuf,
    pub program: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub expected_binary: Option<PathBuf>,
    pub output_directory: PathBuf,
    pub requirements: Vec<String>,
}

pub fn plan(request: &Request, cancel: &Cancellation) -> Result<Plan> {
    let project = Project::load(request, cancel)?;
    make_plan(&project, request)
}
fn make_plan(project: &Project, request: &Request) -> Result<Plan> {
    let host = Host::default();
    ensure!(
        request.platform.os() != "ios" || host.os == "macos",
        "iOS requires a Mac with Xcode; no remote build is performed implicitly"
    );
    let target = project.config.target(request.platform);
    let backend = request.platform.backend(&host);
    let output_directory = request
        .output_dir
        .as_ref()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                project.root.join(p)
            }
        })
        .unwrap_or_else(|| project.root.join("dist/cranpose"))
        .join(request.platform.name())
        .join(request.profile());
    if request.platform == Platform::Android {
        let directory = project.root.join(&project.config.android.directory);
        let wrapper = directory.join("gradle/wrapper/gradle-wrapper.jar");
        ensure!(
            wrapper.is_file(),
            "Android needs the Cranpose Gradle project and gradle-wrapper.jar in {}. Use the Showcase starter or configure android.directory.",
            directory.display()
        );
        let task = format!(
            ":{}:assemble{}",
            project.config.android.module,
            if request.release { "Release" } else { "Debug" }
        );
        let mut arguments = vec![
            "-classpath".into(),
            wrapper.display().to_string(),
            "org.gradle.wrapper.GradleWrapperMain".into(),
            "--no-daemon".into(),
            "--console=plain".into(),
            task,
        ];
        if request.offline {
            arguments.push("--offline".into());
        }
        return Ok(Plan {
            platform: request.platform,
            backend: backend.into(),
            program: "java".into(),
            arguments,
            directory,
            environment: target.env,
            expected_binary: None,
            output_directory,
            requirements: vec![
                "JDK 17+, Android SDK/NDK and accepted SDK licenses".into(),
                "Existing Cranpose Android Gradle wrapper and Cargo lockfile".into(),
            ],
        });
    }
    let binary = project.binary(request)?;
    let mut arguments = match backend {
        "cargo-xwin" => vec!["xwin".into(), "build".into()],
        "cargo-zigbuild" => vec!["zigbuild".into()],
        _ => vec!["build".into()],
    };
    arguments.extend([
        "--locked".into(),
        "--manifest-path".into(),
        project.package.manifest_path.to_string(),
        "--package".into(),
        project.package.name.to_string(),
        "--bin".into(),
        binary.clone(),
        "--target".into(),
        request.platform.triple().into(),
        "--target-dir".into(),
        project.target_dir.display().to_string(),
    ]);
    if request.release {
        arguments.push("--release".into());
    }
    if request.offline {
        arguments.push("--offline".into());
    }
    let mut features = target.features;
    let ios = request.platform.os() == "ios";
    if features.is_empty() && ios && project.package.features.contains_key("ios") {
        features.push("ios".into());
    }
    if target.no_default_features.unwrap_or(ios) {
        arguments.push("--no-default-features".into());
    }
    if !features.is_empty() {
        arguments.extend(["--features".into(), features.join(",")]);
    }
    let filename = if request.platform.os() == "windows" {
        format!("{binary}.exe")
    } else {
        binary
    };
    let expected_binary = project
        .target_dir
        .join(request.platform.triple())
        .join(request.profile())
        .join(filename);
    let mut requirements = vec![format!("Rust target {}", request.platform.triple())];
    if backend == "cargo-xwin" {
        requirements.push("LLVM and cargo-xwin; Microsoft SDK/CRT licenses apply".into());
    }
    if backend == "cargo-zigbuild" {
        requirements.push("Zig, cargo-zigbuild and any native target libraries/sysroot required by the application".into());
    }
    Ok(Plan {
        platform: request.platform,
        backend: backend.into(),
        program: backend.into(),
        arguments,
        directory: project.root.clone(),
        environment: target.env,
        expected_binary: Some(expected_binary),
        output_directory,
        requirements,
    })
}

pub fn build(
    request: &Request,
    cancel: &Cancellation,
    mut emit: impl FnMut(Event),
) -> Result<Artifact> {
    let start = Instant::now();
    emit(Event::Stage {
        message: "Inspecting project".into(),
    });
    let project = Project::load(request, cancel)?;
    let plan = make_plan(&project, request)?;
    let mut command = Command::new(tools::tool(&plan.program)?);
    command
        .args(&plan.arguments)
        .current_dir(&plan.directory)
        .envs(&plan.environment);
    tools::apply_environment(&mut command)?;
    emit(Event::Stage {
        message: format!("Building {} with {}", request.platform.name(), plan.backend),
    });
    process::execute(command, Duration::from_secs(7200), cancel, |text| {
        emit(Event::Log { text: text.into() })
    })?;
    cancel.check()?;
    emit(Event::Stage {
        message: "Packaging application".into(),
    });
    fs::create_dir_all(&plan.output_directory)?;
    let staging = tempfile::Builder::new()
        .prefix("build-")
        .tempdir_in(&plan.output_directory)?;
    let (path, executable) = package(&project, request, &plan, staging.path(), cancel, &mut emit)?;
    let archive = staging.path().join("application.zip");
    archive_path(&path, &archive)?;
    let sha256 = hash(&archive)?;
    let artifact = Artifact {
        platform: request.platform,
        path,
        executable,
        identifier: project.identifier()?,
        archive,
        sha256,
    };
    fs::write(
        staging.path().join("artifact.json"),
        serde_json::to_vec_pretty(&artifact)?,
    )?;
    let _ = staging.keep();
    emit(Event::Artifact {
        artifact: artifact.clone(),
    });
    emit(Event::Complete {
        elapsed_ms: start.elapsed().as_millis(),
    });
    Ok(artifact)
}

fn package(
    project: &Project,
    request: &Request,
    plan: &Plan,
    staging: &Path,
    cancel: &Cancellation,
    emit: &mut impl FnMut(Event),
) -> Result<(PathBuf, Option<PathBuf>)> {
    if request.platform == Platform::Android {
        let root = plan
            .directory
            .join(project.config.android.module.replace(':', "/"))
            .join("build/outputs/apk")
            .join(request.profile());
        let files = files_in(&root)?;
        let apks: Vec<_> = files
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "apk"))
            .collect();
        ensure!(
            apks.len() == 1,
            "Expected one APK in {}; found {}. Configure a single app variant.",
            root.display(),
            apks.len()
        );
        let path = staging.join("application.apk");
        fs::copy(&apks[0], &path)?;
        return Ok((path, None));
    }
    let source = plan
        .expected_binary
        .as_ref()
        .context("Expected executable")?;
    ensure!(
        source.is_file(),
        "Compiler did not produce {}",
        source.display()
    );
    let apple = matches!(request.platform.os(), "macos" | "ios");
    let path = staging.join(if apple {
        "Application.app"
    } else {
        "application"
    });
    let contents = if request.platform.os() == "macos" {
        path.join("Contents")
    } else {
        path.clone()
    };
    let bin_dir = if request.platform.os() == "macos" {
        contents.join("MacOS")
    } else {
        contents.clone()
    };
    fs::create_dir_all(&bin_dir)?;
    let executable = bin_dir.join(source.file_name().context("Executable filename")?);
    fs::copy(source, &executable)?;
    let resources = if request.platform.os() == "macos" {
        contents.join("Resources")
    } else {
        contents.clone()
    };
    fs::create_dir_all(&resources)?;
    for asset in &project.config.app.assets {
        copy_resource(&project.root.join(asset), &resources)?;
    }
    for file in project.config.target(request.platform).runtime_files {
        copy_resource(&project.root.join(file), &bin_dir)?;
    }
    if apple {
        write_plist(project, request, &executable, &contents.join("Info.plist"))?;
        if request.platform.os() == "ios" {
            for asset in &project.config.ios.resources {
                copy_resource(&project.root.join(asset), &resources)?;
            }
            if request.platform == Platform::Ios {
                ensure!(
                    project
                        .config
                        .ios
                        .identity
                        .as_deref()
                        .is_some_and(|s| s != "-"),
                    "Physical iOS devices need ios.identity and ios.provisioning_profile; use ios-sim for ad-hoc development"
                );
                let profile = project
                    .config
                    .ios
                    .provisioning_profile
                    .as_ref()
                    .context("Set ios.provisioning_profile for device builds")?;
                fs::copy(
                    project.root.join(profile),
                    path.join("embedded.mobileprovision"),
                )?;
            }
        }
        if Host::default().os == "macos" {
            let mut sign = Command::new(tools::tool("codesign")?);
            sign.args([
                "--force",
                "--sign",
                project
                    .config
                    .ios
                    .identity
                    .as_deref()
                    .filter(|_| request.platform.os() == "ios")
                    .unwrap_or("-"),
            ]);
            if request.platform.os() == "ios"
                && let Some(entitlements) = &project.config.ios.entitlements
            {
                sign.arg("--entitlements")
                    .arg(project.root.join(entitlements));
            }
            sign.arg(&path);
            process::execute(sign, Duration::from_secs(120), cancel, |text| {
                emit(Event::Log { text: text.into() })
            })?;
        }
    }
    Ok((path, Some(executable)))
}

fn write_plist(
    project: &Project,
    request: &Request,
    executable: &Path,
    destination: &Path,
) -> Result<()> {
    let mut value = if request.platform.os() == "ios" {
        project
            .config
            .ios
            .plist
            .as_ref()
            .map(|p| plist::Value::from_file(project.root.join(p)))
            .transpose()?
    } else {
        None
    }
    .unwrap_or_else(|| plist::Value::Dictionary(Default::default()));
    let dict = value
        .as_dictionary_mut()
        .context("Info.plist root must be a dictionary")?;
    for (key, value) in [
        (
            "CFBundleExecutable",
            executable
                .file_name()
                .context("Executable filename")?
                .to_string_lossy()
                .into_owned(),
        ),
        ("CFBundleIdentifier", project.identifier()?),
        ("CFBundleName", project.app_name().into()),
        ("CFBundleDisplayName", project.app_name().into()),
        ("CFBundlePackageType", "APPL".into()),
        (
            "CFBundleShortVersionString",
            project.package.version.to_string(),
        ),
    ] {
        dict.insert(key.into(), value.into());
    }
    if !dict.contains_key("CFBundleVersion") {
        dict.insert("CFBundleVersion".into(), "1".into());
    }
    if request.platform.os() == "ios" {
        dict.insert("LSRequiresIPhoneOS".into(), true.into());
        for (key, value) in [
            ("MinimumOSVersion", "16.0".into()),
            (
                "UILaunchScreen",
                plist::Value::Dictionary(Default::default()),
            ),
            (
                "UIDeviceFamily",
                plist::Value::Array(vec![1i64.into(), 2i64.into()]),
            ),
        ] {
            if !dict.contains_key(key) {
                dict.insert(key.into(), value);
            }
        }
    }
    value.to_file_xml(destination)?;
    Ok(())
}

pub fn files_in(path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("Read {}", path.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "Symlink resources must be materialized explicitly: {}",
            entry.path().display()
        );
        if kind.is_dir() {
            files.extend(files_in(&entry.path())?);
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}
fn copy_resource(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        !source.symlink_metadata()?.file_type().is_symlink(),
        "Symlink resource {}",
        source.display()
    );
    let root = destination.join(source.file_name().context("Resource name")?);
    if source.is_dir() {
        fs::create_dir_all(&root)?;
        for file in files_in(source)? {
            let out = root.join(file.strip_prefix(source)?);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(file, out)?;
        }
    } else {
        fs::copy(source, root)?;
    }
    Ok(())
}
pub fn archive_path(source: &Path, destination: &Path) -> Result<()> {
    let base = source.parent().context("Archive source directory")?;
    let files = if source.is_dir() {
        files_in(source)?
    } else {
        vec![source.to_owned()]
    };
    let mut zip = zip::ZipWriter::new(fs::File::create(destination)?);
    for path in files {
        let name = path
            .strip_prefix(base)?
            .to_string_lossy()
            .replace('\\', "/");
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()?.permissions().mode()
        };
        #[cfg(not(unix))]
        let permissions = 0o755;
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(permissions),
        )?;
        std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
    }
    zip.finish()?.flush()?;
    Ok(())
}
pub fn hash(path: &Path) -> Result<String> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut file = fs::File::open(path)?;
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn run(
    artifact: &Artifact,
    device: Option<&str>,
    cancel: &Cancellation,
    mut emit: impl FnMut(Event),
) -> Result<()> {
    let host = Host::default();
    let mut commands = Vec::new();
    if artifact.platform.simulator() {
        ensure!(host.os == "macos", "iOS simulators run on macOS");
        let device = device
            .context("Select a simulator with --device <UDID>; use devices --platform ios-sim")?;
        let mut install = Command::new(tools::tool("xcrun")?);
        install
            .args(["simctl", "install", device])
            .arg(&artifact.path);
        commands.push(install);
        let mut launch = Command::new(tools::tool("xcrun")?);
        launch.args(["simctl", "launch", device, &artifact.identifier]);
        commands.push(launch);
    } else if artifact.platform == Platform::Ios {
        ensure!(host.os == "macos", "iOS device deployment requires Xcode");
        let device = device.context("Select an iOS device with --device <UDID>")?;
        let mut install = Command::new(tools::tool("xcrun")?);
        install
            .args(["devicectl", "device", "install", "app", "--device", device])
            .arg(&artifact.path);
        commands.push(install);
        let mut launch = Command::new(tools::tool("xcrun")?);
        launch.args([
            "devicectl",
            "device",
            "process",
            "launch",
            "--device",
            device,
            &artifact.identifier,
        ]);
        commands.push(launch);
    } else if artifact.platform == Platform::Android {
        let device = device.context(
            "Select an Android device with --device <serial>; use devices --platform android",
        )?;
        let mut install = Command::new(tools::tool("adb")?);
        install
            .args(["-s", device, "install", "-r"])
            .arg(&artifact.path);
        commands.push(install);
        let mut launch = Command::new(tools::tool("adb")?);
        launch.args([
            "-s",
            device,
            "shell",
            "monkey",
            "-p",
            &artifact.identifier,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ]);
        commands.push(launch);
    } else {
        ensure!(
            artifact.platform.os() == host.os,
            "{} binaries need a matching OS, VM or explicit remote machine to run",
            artifact.platform.name()
        );
        let executable = artifact
            .executable
            .as_ref()
            .context("No executable recorded")?;
        let mut command = Command::new(executable);
        command.current_dir(executable.parent().context("Executable directory")?);
        commands.push(command);
    }
    for command in commands {
        process::execute(command, Duration::from_secs(86400), cancel, |text| {
            emit(Event::Log { text: text.into() })
        })?;
    }
    Ok(())
}

pub fn devices(platform: Platform, cancel: &Cancellation) -> Result<String> {
    let mut command;
    if platform == Platform::Android {
        command = Command::new(tools::tool("adb")?);
        command.args(["devices", "-l"]);
    } else if platform.simulator() {
        command = Command::new(tools::tool("xcrun")?);
        command.args(["simctl", "list", "devices", "available", "--json"]);
    } else if platform == Platform::Ios {
        command = Command::new(tools::tool("xcrun")?);
        command.args(["devicectl", "list", "devices"]);
    } else {
        bail!(
            "{} uses a native OS; choose android, ios or ios-sim for devices",
            platform.name()
        );
    }
    process::capture(command, cancel)
}
