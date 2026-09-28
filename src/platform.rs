use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Mac,
    MacIntel,
    Linux,
    LinuxArm,
    Windows,
    WindowsArm,
    Android,
    Ios,
    IosSim,
    IosSimIntel,
}

impl Platform {
    pub const ALL: [Self; 10] = [
        Self::Mac,
        Self::MacIntel,
        Self::Linux,
        Self::LinuxArm,
        Self::Windows,
        Self::WindowsArm,
        Self::Android,
        Self::Ios,
        Self::IosSim,
        Self::IosSimIntel,
    ];
    pub fn triple(self) -> &'static str {
        match self {
            Self::Mac => "aarch64-apple-darwin",
            Self::MacIntel => "x86_64-apple-darwin",
            Self::Linux => "x86_64-unknown-linux-gnu",
            Self::LinuxArm => "aarch64-unknown-linux-gnu",
            Self::Windows => "x86_64-pc-windows-msvc",
            Self::WindowsArm => "aarch64-pc-windows-msvc",
            Self::Android => "aarch64-linux-android",
            Self::Ios => "aarch64-apple-ios",
            Self::IosSim => "aarch64-apple-ios-sim",
            Self::IosSimIntel => "x86_64-apple-ios",
        }
    }
    pub fn os(self) -> &'static str {
        match self {
            Self::Mac | Self::MacIntel => "macos",
            Self::Linux | Self::LinuxArm => "linux",
            Self::Windows | Self::WindowsArm => "windows",
            Self::Android => "android",
            Self::Ios | Self::IosSim | Self::IosSimIntel => "ios",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Mac => "mac",
            Self::MacIntel => "mac-intel",
            Self::Linux => "linux",
            Self::LinuxArm => "linux-arm",
            Self::Windows => "windows",
            Self::WindowsArm => "windows-arm",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::IosSim => "ios-sim",
            Self::IosSimIntel => "ios-sim-intel",
        }
    }
    pub fn simulator(self) -> bool {
        matches!(self, Self::IosSim | Self::IosSimIntel)
    }
    pub fn backend(self, host: &Host) -> &'static str {
        if self == Self::Android {
            "gradle"
        } else if self.os() == "windows" && host.os != "windows" {
            "cargo-xwin"
        } else if (self.os() == "linux"
            && (host.os != "linux" || !self.triple().starts_with(&host.arch)))
            || (self.os() == "macos" && host.os != "macos")
        {
            "cargo-zigbuild"
        } else {
            "cargo"
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub os: String,
    pub arch: String,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        }
    }
}
impl Host {
    pub fn native(&self) -> Platform {
        match (self.os.as_str(), self.arch.as_str()) {
            ("macos", "aarch64") => Platform::Mac,
            ("macos", _) => Platform::MacIntel,
            ("windows", "aarch64") => Platform::WindowsArm,
            ("windows", _) => Platform::Windows,
            ("linux", "aarch64") => Platform::LinuxArm,
            _ => Platform::Linux,
        }
    }
}
