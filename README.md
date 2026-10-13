# Cranpose Build

A Rust CLI and library for cross-platform builds, packaging and device launch.
Applications build on your computer. Apple platforms can also build on a GitHub
macOS runner when you pass `--on github`, so a Linux or Windows computer can make
macOS and iOS packages.

```console
cranpose-build doctor
cranpose-build setup --platform linux
cranpose-build plan --platform windows
cranpose-build build --platform windows --release
cranpose-build devices --platform android
cranpose-build run --platform android --device emulator-5554
cranpose-build run --platform ios-sim --device <simulator-UDID>
cranpose-build init-ci
cranpose-build build --platform mac --on github
cranpose-build run --platform ios --on github --device <iPhone-UDID>
```

## Platforms

| Application | Build backend | Package and launch |
|---|---|---|
| macOS ARM64 / Intel | Native Cargo; a GitHub macOS runner with `--on github`; cargo-zigbuild with a user-provided Apple SDK elsewhere | `.app`, ad-hoc signing on macOS, native launch |
| Linux x64 / ARM64 | Native Cargo or cargo-zigbuild with Zig and required target libraries | Executable, assets and configured runtime libraries; native launch |
| Windows x64 / ARM64 | Native MSVC Cargo or cargo-xwin with LLVM and Microsoft SDK/CRT | Executable, assets and configured runtime libraries; native launch |
| Android | Existing Cranpose Gradle project, SDK and NDK | APK, adb install and launch |
| iOS devices / simulators | Cargo and Xcode on macOS, or a GitHub macOS runner with `--on github` | `.app` and device `.ipa`; Xcode device/simulator install and launch on macOS; xtool device install elsewhere |

Foreign applications still need their OS, VM or device to run. The iOS simulator
runs only on macOS. Apple SDKs are not redistributed. Native dependencies
may require a target sysroot: `doctor` checks base tools, while compilation reports
project-specific missing libraries. A desktop package is not automatically
static, notarized or store-ready.

## Apple apps from Linux or Windows

Apple's SDK license allows the SDK only on Apple computers, so macOS and iOS
code compiles on a Mac. GitHub-hosted macOS runners are free for public
repositories; private repositories spend the account's Actions minutes.

1. Push the project to GitHub with `origin` as its remote, and sign in with the
   [GitHub CLI](https://cli.github.com) (`gh auth login`).
2. Run `cranpose-build init-ci`. It writes `.github/workflows/cranpose-build.yml`,
   which calls this repository's [Apple workflow](.github/workflows/apple.yml).
   Commit the file to the default branch and push it.
3. Run `cranpose-build build --platform ios --on github`. The tool commits the
   working tree, including untracked files that Git does not ignore, to a
   temporary `cranpose-build/*` branch without changing your index or files. It
   starts the workflow on that branch, waits for the run, downloads the package
   into `dist/cranpose/<platform>/<profile>` and deletes the branch. Ctrl+C
   cancels the run. The snapshot commit says `[skip ci]`, so push-triggered
   workflows skip it.

The run keeps the package as a workflow artifact for 14 days. Platforms are
`mac`, `mac-intel`, `ios`, `ios-sim` and `ios-sim-intel`.

A macOS package is an ad-hoc signed `.app`. Another Mac opens it after its owner
chooses **Open Anyway** in **System Settings > Privacy & Security**. Notarization,
which removes that step, needs a paid Apple Developer Program membership.

An iOS device build without `ios.identity` is an `.ipa` that a device installs
only after a signing tool signs it for that device. On Linux, or on Windows inside
WSL, `cranpose-build run --platform ios --device <UDID>` and `launch` call
[xtool](https://xtool.sh), which signs the app with your Apple account and installs
it over USB. `devices --platform ios` lists connected iPhones through xtool. With a
free Apple ID, the app expires after 7 days and the account can install 3 apps.
xtool registers the app under its own bundle ID, so open it on the device. Other
people install the same `.ipa` by signing it with their own Apple account, for
example with AltStore, SideStore or xtool. Sharing a signed app through TestFlight
or the App Store needs a paid membership.

## Install and setup

Download the ZIP for your OS and architecture from
[Releases](https://github.com/samoylenkodmitry/cranpose-build/releases/latest),
verify its accompanying SHA256, and extract `cranpose-build` (`.exe` on Windows).
Prebuilt downloads cover ARM64 and x86-64 macOS, Linux and Windows and do not
require Git. Put the executable on PATH or invoke it by its full path.

Alternatively, install from source with Cargo:

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
# A signed device build needs an identity and a matching provisioning profile.
# Without them, the device build is an IPA for xtool, AltStore or SideStore.
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
application, a ZIP (an IPA for iOS devices), its SHA256 and `artifact.json`. Use `--target-dir` for Cargo caches
and `--output-dir` for packages. Failures do not publish successful artifacts.
The build directory is portable: copy it to the matching OS and run
`cranpose-build launch --artifact /path/to/build-directory/artifact.json`.
Manifest paths are relative to that directory; moving a package does not require
the original developer's checkout or home directory.

`--json` provides typed events for IDE integration. The Rust library exposes the
same planner, build, GitHub build, launch, doctor and cancellation APIs. Ctrl+C stops the owned
subprocess tree using the same Rust process SDK as Cranpose Studio.

Desktop launch reports `Application running` after successful OS process creation,
then `Application exited` after a successful exit. A failed spawn never reports
running. This status does not promise a rendered frame or application readiness.
Mobile tools retain `Application launched` after their successful device handoff;
their launcher process is not the application process.

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
