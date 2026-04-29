use std::ops::Deref;

use anyhow::anyhow;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)] // requires `derive` feature
#[command(name = "cargo-xtask")]
#[command(about = "Helpful commands for working with this repo", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build,
    Package,
}

const DEFAULT_TARGETS: &[&str] = &["mipsel-unknown-linux-musl"];

fn main() -> anyhow::Result<()> {
    let input = Cli::parse();
    match input.command {
        Command::Build => build(DEFAULT_TARGETS.into_iter().map(Deref::deref))?,
        Command::Package => todo!("package"),
    }

    Ok(())
}

fn build<'a>(targets: impl IntoIterator<Item = &'a str>) -> anyhow::Result<()> {
    let cross_installed = which::which("cross").is_ok();
    if !cross_installed {
        return Err(anyhow!(
            "Please install Cross first: https://crates.io/crates/cross"
        ));
    }

    let handles = targets
        .into_iter()
        .map(|t| {
            duct::cmd("cross", &["build", "--package", "dorfconf", "--target", t])
                .stdout_to_stderr()
                .stderr_capture()
                .unchecked()
                .start()
                .map(|h| (t, h))
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (target, handle) in handles {
        eprintln!("Target: {target}");
        let out = handle.wait()?;
        if out.status.success() {
            eprintln!("✅ Built!")
        } else {
            eprintln!(
                "⚠️ Failed to build, exit {}",
                out.status.code().unwrap_or(0)
            );
            eprintln!("{}", String::from_utf8_lossy(&out.stderr));
        }
    }

    Ok(())
}
