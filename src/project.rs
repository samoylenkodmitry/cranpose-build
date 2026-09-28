use crate::{Cancellation, Config, Request, process, tools};
use anyhow::{Context, Result, ensure};
use cargo_metadata::{Metadata, Package};
use std::{path::PathBuf, process::Command};

pub struct Project {
    pub package: Package,
    pub root: PathBuf,
    pub target_dir: PathBuf,
    pub config: Config,
}
impl Project {
    pub fn load(request: &Request, cancel: &Cancellation) -> Result<Self> {
        let manifest = request.manifest.canonicalize().context("Find Cargo.toml")?;
        let mut command =
            Command::new(tools::find("cargo").context(
                "Cargo is missing. Install Rust from https://rustup.rs, then run doctor.",
            )?);
        command
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
            ])
            .arg(&manifest);
        if request.offline {
            command.arg("--offline");
        }
        let metadata: Metadata = serde_json::from_str(&process::capture(command, cancel)?)?;
        let package = if let Some(name) = &request.package {
            metadata
                .packages
                .iter()
                .find(|p| p.name.as_str() == name)
                .with_context(|| format!("Package {name} is not in this workspace"))?
        } else if let Some(package) = metadata.root_package() {
            package
        } else {
            let packages: Vec<_> = metadata
                .packages
                .iter()
                .filter(|p| metadata.workspace_default_members.contains(&p.id))
                .collect();
            ensure!(
                packages.len() == 1,
                "Choose a workspace package with --package"
            );
            packages[0]
        }
        .clone();
        let root = package
            .manifest_path
            .parent()
            .context("Package directory")?
            .as_std_path()
            .to_owned();
        let config = Config::read(&root)?;
        let target_dir = request
            .target_dir
            .as_ref()
            .map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    root.join(p)
                }
            })
            .unwrap_or_else(|| metadata.target_directory.into_std_path_buf());
        Ok(Self {
            package,
            root,
            target_dir,
            config,
        })
    }
    pub fn binary(&self, request: &Request) -> Result<String> {
        let target = self.config.target(request.platform);
        let binaries: Vec<_> = self.package.targets.iter().filter(|t| t.is_bin()).collect();
        if let Some(name) = request.binary.clone().or(target.bin) {
            ensure!(
                binaries.iter().any(|t| t.name == name),
                "Unknown binary {name}"
            );
            return Ok(name);
        }
        if request.platform.os() == "ios" {
            let ios: Vec<_> = binaries
                .iter()
                .filter(|t| t.required_features.iter().any(|f| f == "ios"))
                .collect();
            if ios.len() == 1 {
                return Ok(ios[0].name.clone());
            }
        } else {
            if let Some(name) = &self.package.default_run {
                return Ok(name.clone());
            }
            if let Some(target) = binaries
                .iter()
                .find(|t| t.name == self.package.name.as_str())
            {
                return Ok(target.name.clone());
            }
        }
        ensure!(
            binaries.len() == 1,
            "Set targets.{}.bin in CranposeBuild.toml to choose a binary",
            request.platform.name()
        );
        Ok(binaries[0].name.clone())
    }
    pub fn app_name(&self) -> &str {
        self.config
            .app
            .name
            .as_deref()
            .unwrap_or(self.package.name.as_str())
    }
    pub fn identifier(&self) -> Result<String> {
        let id = self
            .config
            .app
            .identifier
            .clone()
            .unwrap_or_else(|| format!("dev.cranpose.{}", self.package.name.replace('-', "")));
        ensure!(
            id.split('.').all(|part| !part.is_empty()
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                && id.contains('.'),
            "Invalid application identifier"
        );
        Ok(id)
    }
}
