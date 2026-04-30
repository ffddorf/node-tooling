use clap::{Parser, Subcommand};

mod build;
use build::build;

mod package;
use package::package;

mod types;
use types::{OpenWrtArch, Profile};

#[derive(Debug, Parser)] // requires `derive` feature
#[command(name = "cargo-xtask")]
#[command(about = "Helpful commands for working with this repo", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[arg(short, long, global = true)]
    profile: Option<String>,
    #[arg(long, global = true)]
    release: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build,
    Package,
}

const DEFAULT_ARCHS: &[OpenWrtArch] = &[OpenWrtArch::Mipsel24kc];

fn main() -> anyhow::Result<()> {
    let input = Cli::parse();

    // todo: take override from command line arg
    let archs = DEFAULT_ARCHS;
    let profile = match input.profile {
        Some(prof) => Profile::Custom(prof),
        None => {
            if input.release {
                Profile::Release
            } else {
                Profile::Debug
            }
        }
    };

    match input.command {
        Command::Build => {
            let target_num = build(archs.iter().copied(), profile)?;
            if target_num == 0 {
                eprintln!("Warning: No targets built!")
            }
        }
        Command::Package => package(archs.iter().copied(), profile)?,
    }

    Ok(())
}
