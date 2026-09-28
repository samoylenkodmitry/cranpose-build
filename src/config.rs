use crate::Platform;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub app: App,
    pub targets: BTreeMap<String, Target>,
    pub android: Android,
    pub ios: Ios,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct App {
    pub name: Option<String>,
    pub identifier: Option<String>,
    pub assets: Vec<PathBuf>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Target {
    pub bin: Option<String>,
    pub features: Vec<String>,
    pub no_default_features: Option<bool>,
    /// Explicit target SDK / sysroot settings, applied only to the child process.
    pub env: BTreeMap<String, String>,
    pub runtime_files: Vec<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Android {
    pub directory: PathBuf,
    pub module: String,
    pub properties: BTreeMap<String, String>,
}
impl Default for Android {
    fn default() -> Self {
        Self {
            directory: "android".into(),
            module: "app".into(),
            properties: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ios {
    pub plist: Option<PathBuf>,
    pub resources: Vec<PathBuf>,
    pub identity: Option<String>,
    pub provisioning_profile: Option<PathBuf>,
    pub entitlements: Option<PathBuf>,
}

impl Config {
    pub fn read(root: &Path) -> Result<Self> {
        let path = root.join("CranposeBuild.toml");
        let config: Self = match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("Read {}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        for key in config.targets.keys() {
            ensure!(
                Platform::ALL
                    .iter()
                    .any(|p| p.name() == key || p.os() == key),
                "Unknown platform {key}"
            );
        }
        ensure!(
            !config.android.module.is_empty()
                && config
                    .android
                    .module
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-:".contains(c)),
            "Invalid Android module"
        );
        Ok(config)
    }
    pub fn target(&self, platform: Platform) -> Target {
        self.targets
            .get(platform.name())
            .or_else(|| self.targets.get(platform.os()))
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub manifest: PathBuf,
    pub package: Option<String>,
    #[serde(default)]
    pub binary: Option<String>,
    pub platform: Platform,
    pub release: bool,
    pub offline: bool,
    pub target_dir: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub device: Option<String>,
}
impl Request {
    pub fn new(manifest: impl Into<PathBuf>, platform: Platform) -> Self {
        Self {
            manifest: manifest.into(),
            platform,
            package: None,
            binary: None,
            release: false,
            offline: false,
            target_dir: None,
            output_dir: None,
            device: None,
        }
    }
    pub fn profile(&self) -> &'static str {
        if self.release { "release" } else { "debug" }
    }
}
