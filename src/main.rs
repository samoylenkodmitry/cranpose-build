use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use cranpose_build::{Cancellation, Event, Host, Platform, Request, engine, tools};
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Build and launch Cranpose apps with local platform toolchains"
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Explain toolchain readiness. No downloads or changes.
    Doctor {
        #[arg(long)]
        platform: Option<Platform>,
    },
    /// Install Rust targets and pinned Cargo backends in a private directory.
    Setup {
        #[arg(long)]
        platform: Platform,
    },
    /// Show the exact local command without compiling.
    Plan(BuildArgs),
    /// Compile and package an application. No remote or CI fallback.
    Build(BuildArgs),
    /// Compile, package, then launch on the native OS or selected device.
    Run(BuildArgs),
    /// List Android devices or iOS devices/simulators.
    Devices {
        #[arg(long)]
        platform: Platform,
    },
    /// Launch a previously built artifact.json without compiling again.
    Launch {
        artifact: PathBuf,
        #[arg(long)]
        device: Option<String>,
    },
}
#[derive(Args)]
struct BuildArgs {
    #[arg(long, default_value = "Cargo.toml")]
    manifest_path: PathBuf,
    #[arg(long)]
    package: Option<String>,
    #[arg(long)]
    platform: Option<Platform>,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long)]
    target_dir: Option<PathBuf>,
    #[arg(long)]
    output_dir: Option<PathBuf>,
    #[arg(long)]
    device: Option<String>,
}
impl From<BuildArgs> for Request {
    fn from(a: BuildArgs) -> Self {
        Self {
            manifest: a.manifest_path,
            package: a.package,
            platform: a.platform.unwrap_or_else(|| Host::default().native()),
            release: a.release,
            offline: a.offline,
            target_dir: a.target_dir,
            output_dir: a.output_dir,
            device: a.device,
        }
    }
}
fn emit(json: bool, event: Event) {
    if json {
        if let Ok(text) = serde_json::to_string(&event) {
            println!("{text}");
            let _ = std::io::stdout().flush();
        }
    } else {
        match event {
            Event::Stage { message } => eprintln!("{message}"),
            Event::Log { text } => eprintln!("{text}"),
            Event::Artifact { artifact } => println!(
                "{}\nArchive: {}\nSHA256: {}",
                artifact.path.display(),
                artifact.archive.display(),
                artifact.sha256
            ),
            Event::Complete { elapsed_ms } => {
                eprintln!("Finished in {:.2}s", elapsed_ms as f64 / 1000.0)
            }
            Event::Error { message } => eprintln!("{message}"),
        }
    }
}
fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let cancel = Cancellation::default();
    let signal = cancel.clone();
    if let Err(error) = ctrlc::set_handler(move || signal.cancel()) {
        eprintln!("Install cancellation handler: {error}");
        return std::process::ExitCode::FAILURE;
    }
    let json = cli.json;
    let result = execute(cli, &cancel);
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            emit(
                json,
                Event::Error {
                    message: format!("{error:#}"),
                },
            );
            std::process::ExitCode::FAILURE
        }
    }
}
fn execute(cli: Cli, cancel: &Cancellation) -> Result<()> {
    match cli.command {
        Action::Doctor { platform } => {
            let platforms = platform
                .map(|p| vec![p])
                .unwrap_or_else(|| Platform::ALL.to_vec());
            let results: Vec<_> = platforms
                .into_iter()
                .map(|p| tools::doctor(p, cancel))
                .collect();
            if cli.json {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                for d in &results {
                    println!(
                        "{} — {}",
                        d.platform.name(),
                        if d.ready { "ready" } else { "setup needed" }
                    );
                    for c in &d.checks {
                        println!(
                            "  {} {}: {}",
                            if c.ready { "+" } else { "!" },
                            c.name,
                            c.detail
                        );
                    }
                    println!("  {}", d.launch);
                }
            }
        }
        Action::Setup { platform } => tools::setup(platform, cancel, |text| {
            emit(cli.json, Event::Log { text: text.into() })
        })?,
        Action::Plan(args) => println!(
            "{}",
            serde_json::to_string_pretty(&engine::plan(&args.into(), cancel)?)?
        ),
        Action::Build(args) => {
            engine::build(&args.into(), cancel, |event| emit(cli.json, event))?;
        }
        Action::Run(args) => {
            let request = args.into();
            let artifact = engine::build(&request, cancel, |event| emit(cli.json, event))?;
            engine::run(&artifact, request.device.as_deref(), cancel, |event| {
                emit(cli.json, event)
            })?;
        }
        Action::Devices { platform } => println!("{}", engine::devices(platform, cancel)?),
        Action::Launch { artifact, device } => {
            let artifact = serde_json::from_slice(&std::fs::read(artifact)?)?;
            engine::run(&artifact, device.as_deref(), cancel, |event| {
                emit(cli.json, event)
            })?;
        }
    }
    Ok(())
}
