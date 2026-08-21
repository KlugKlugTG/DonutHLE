use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use donuthle::{apk, runtime::Runtime};

#[derive(Parser, Debug)]
#[command(name = "donuthle", version, about = "Android 1.x HLE prototype")]
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
    let raw_args: Vec<OsString> = std::env::args_os().collect();
    let implicit_launch = raw_args.len() == 1;
    let cli = if implicit_launch {
        #[cfg(windows)]
        {
            match sibling_apk() {
                Some(path) => Cli {
                    command: Command::Run { apk: path },
                },
                None => {
                    print_startup_help();
                    pause_after_explorer_launch();
                    return Ok(());
                }
            }
        }
        #[cfg(not(windows))]
        {
            print_startup_help();
            return Ok(());
        }
    } else if raw_args.len() == 2 && is_apk_path(&raw_args[1]) {
        Cli {
            command: Command::Run {
                apk: PathBuf::from(&raw_args[1]),
            },
        }
    } else {
        Cli::parse()
    };
    let mut runtime = Runtime::default();
    let result: Result<()> = (|| {
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
                println!("valid Android 1.x package");
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
                println!("launcher: {}", report.launcher_activity);
                println!("dex: {}", report.dex);
                for line in report.compatibility.format_lines() {
                    println!("{line}");
                }
                #[cfg(target_os = "linux")]
                if std::env::var_os("DISPLAY").is_some() {
                    donuthle::desktop::present(runtime)?;
                } else {
                    println!("Linux framebuffer rendered; no X11 DISPLAY is available for a native window");
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        if implicit_launch {
            eprintln!("Runtime error: {error}");
            #[cfg(windows)]
            pause_after_explorer_launch();
            return Ok(());
        }
        return Err(error);
    }
    if implicit_launch {
        #[cfg(windows)]
        pause_after_explorer_launch();
    }
    Ok(())
}

fn is_apk_path(path: &OsString) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("apk"))
}

#[cfg(windows)]
fn sibling_apk() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let mut apks = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("apk")
                })
        });
    let first = apks.next()?;
    if apks.next().is_none() {
        Some(first)
    } else {
        None
    }
}

fn print_startup_help() {
    let mut command = Cli::command();
    let _ = command.print_help();
    println!();
    println!("Drag an APK file onto DonutHLE.exe, or run:");
    println!("  DonutHLE.exe run path\\\\to\\\\game.apk");
}

#[cfg(windows)]
fn pause_after_explorer_launch() {
    use std::io::{self, Write};

    print!("\\nPress Enter to close this window...");
    let _ = io::stdout().flush();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}
