use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use donuthle::{apk, runtime::Runtime};

#[derive(Parser, Debug)]
#[command(
    name = "donuthle",
    version,
    about = "Android 1.6 (Donut) HLE prototype"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Inspect { apk: PathBuf },
    Validate { apk: PathBuf },
    Run { apk: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut runtime = Runtime::default();
    match cli.command {
        Command::Inspect { apk: path } => {
            let info = apk::inspect(&path)?;
            println!("{}", info.path);
            println!("entries: {}", info.entries.len());
            println!("manifest: {}", info.has_manifest);
            println!("classes.dex: {}", info.has_dex);
            for entry in info.entries {
                println!("  {entry}");
            }
        }
        Command::Validate { apk: path } => {
            let report = runtime.validate_apk(&path)?;
            println!("valid Donut package");
            println!("package: {}", report.package);
            println!("dex: {}", report.dex);
            println!("status: {}", report.message);
            for line in report.compatibility.format_lines() {
                println!("{line}");
            }
        }
        Command::Run { apk: path } => {
            let report = runtime.launch(&path)?;
            println!("{}", report.message);
            for line in report.compatibility.format_lines() {
                println!("{line}");
            }
        }
    }
    Ok(())
}
