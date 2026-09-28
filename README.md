# Cranpose Build

A Rust CLI and library for local cross-platform builds, packaging and device
launch. GitHub Actions tests and distributes the tool; your applications build
on your computer.

```console
cranpose-build doctor
cranpose-build setup --platform linux
cranpose-build plan --platform windows
cranpose-build build --platform windows --release
cranpose-build devices --platform android
cranpose-build run --platform android --device emulator-5554
cranpose-build run --platform ios-sim --device <simulator-UDID>
```

## Platforms

| Application | Build backend | Package and launch |
|---|---|---|
| macOS ARM64 / Intel | Native Cargo; cargo-zigbuild with a user-provided Apple SDK elsewhere | `.app`, ad-hoc signing on macOS, native launch |
| Linux x64 / ARM64 | Native Cargo or cargo-zigbuild with Zig and required target libraries | Executable, assets and configured runtime libraries; native launch |
| Windows x64 / ARM64 | Native MSVC Cargo or cargo-xwin with LLVM and Microsoft SDK/CRT | Executable, assets and configured runtime libraries; native launch |
| Android | Existing Cranpose Gradle project, SDK and NDK | APK, adb install and launch |
| iOS devices / simulators | Cargo and Xcode on macOS | `.app`, signing, Xcode device/simulator install and launch |

Foreign applications still need their OS, VM or device to run. iOS packaging and
launch require macOS/Xcode. Apple SDKs are not redistributed. Native dependencies
may require a target sysroot: `doctor` checks base tools, while compilation reports
project-specific missing libraries. A desktop package is not automatically
static, notarized or store-ready.

## Install and setup

```console
cargo install --locked --git https://github.com/samoylenkodmitry/cranpose-build
```

`setup` installs the Rust target and pinned Cargo backend in `~/.cranpose-build`
(override with `CRANPOSE_BUILD_HOME`). Zig builds also get Zig 0.15.2 from its
official server, checked against its published SHA256. Setup does not install Git,
replace Rust, change shell profiles or accept SDK licenses. Rust, platform SDKs,
LLVM/MSVC and a JDK remain prerequisites; `doctor` provides their setup guidance.
`LLVM_HOME`, `JAVA_HOME`, `ANDROID_HOME`, `SDKROOT` and Cargo's environment variables
are supported. SDK downloads require network access; builds can use `--offline`
after Cargo/Gradle dependencies are cached.

## Project configuration

Use the package directory or `--manifest-path` and `--package`. A simple desktop
binary needs no configuration. Optional `CranposeBuild.toml` beside Cargo.toml:

```toml
[app]
name = "My App"
identifier = "dev.example.myapp"
assets = ["assets"]

[targets.ios]
bin = "myapp-ios"
features = ["ios"]
no_default_features = true

[targets.linux.env]
PKG_CONFIG_SYSROOT_DIR = "/opt/my-linux-sysroot"

[android]
directory = "android"
module = "app"

# Pass existing Gradle project properties without modifying build scripts.
[android.properties]
showcaseAbi = "arm64-v8a"

[ios]
plist = "ios/Info.plist"
resources = ["ios/AppIcon.png"]
# Physical devices need a real identity and matching provisioning profile.
# identity = "Apple Development: ..."
# provisioning_profile = "local/development.mobileprovision"
# entitlements = "local/entitlements.plist"
```

Targets are `mac`, `mac-intel`, `linux`, `linux-arm`, `windows`, `windows-arm`,
`android`, `ios`, `ios-sim` and `ios-sim-intel`. OS configuration, such as
`targets.ios`, also applies to its variants. Desktop builds keep existing default
features. iOS selects the package's `ios` feature with defaults disabled unless
overridden. `runtime_files` can copy explicitly selected shared libraries beside
the executable; linkage remains the application's concern.

Builds use Cargo.lock with `--locked`. Source files and application release
configuration are unchanged. No hot-reload instrumentation is injected.
Outputs are isolated beneath `dist/cranpose/<platform>/<profile>`, including the
application, ZIP, SHA256 and `artifact.json`. Use `--target-dir` for Cargo caches
and `--output-dir` for packages. Failures do not publish successful artifacts.
The build directory is portable: copy it to the matching OS and run
`cranpose-build launch --artifact /path/to/build-directory/artifact.json`.
Manifest paths are relative to that directory; moving a package does not require
the original developer's checkout or home directory.

`--json` provides typed events for IDE integration. The Rust library exposes the
same planner, build, launch, doctor and cancellation APIs. Ctrl+C stops the owned
subprocess tree using the same Rust process SDK as Cranpose Studio.

## Development

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests compile/run a fixture, inspect packages, handle paths with spaces, preserve
Cargo configuration and reject failed builds, invalid configuration and bad
checksums. The template SDK tests streaming, cancellation and descendant exit on
three desktop OSes. Compilation alone does not establish GUI/runtime support.

The Android package identifier is read from Gradle's output metadata. Its signing
configuration and available ABIs remain those of the Android project.

## Credits

[Cranpose](https://github.com/samoylenkodmitry/Cranpose),
[Cranpose plugin SDK](https://github.com/samoylenkodmitry/cranpose-intellij-plugin-template),
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild),
[cargo-xwin](https://github.com/rust-cross/cargo-xwin), [Zig](https://ziglang.org),
Rust, LLVM, Android SDK/NDK and Xcode provide the underlying platform tools.
Their licenses continue to apply.
