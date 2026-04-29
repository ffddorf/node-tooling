use std::{
    fs::{self, create_dir_all},
    ops::Deref,
    path::PathBuf,
};

use anyhow::{Context, anyhow};
use clap::{Parser, Subcommand};
use tempdir::TempDir;
use wolfpack::ipk::{self, Package, PackageSigner};

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

    // todo: take override from command line arg
    let targets = DEFAULT_TARGETS.into_iter().map(Deref::deref);
    let profile = "debug";

    match input.command {
        Command::Build => {
            let target_num = build(targets, profile)?;
            if target_num == 0 {
                eprintln!("Warning: No targets built!")
            }
        }
        Command::Package => package(targets, profile)?,
    }

    Ok(())
}

fn build<'a>(
    targets: impl IntoIterator<Item = &'a str>,
    profile: &'a str,
) -> anyhow::Result<usize> {
    let cross_installed = which::which("cross").is_ok();
    if !cross_installed {
        return Err(anyhow!(
            "Please install Cross first: https://crates.io/crates/cross"
        ));
    }

    let handles = targets
        .into_iter()
        .map(|t| {
            duct::cmd(
                "cross",
                &[
                    "build",
                    "--package",
                    "dorfconf",
                    "--profile",
                    profile,
                    "--target",
                    t,
                ],
            )
            .stdout_to_stderr()
            .stderr_capture()
            .unchecked()
            .start()
            .map(|h| (t, h))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let target_num = handles.len();
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

    Ok(target_num)
}

fn target_to_arch(target: &str) -> anyhow::Result<ipk::Arch> {
    use wolfpack::ipk::Arch;
    let arch = match target {
        "mipsel-unknown-linux-musl" => Arch::Mipsel,
        _ => return Err(anyhow!("Unable to turn target into arch: {}", target)),
    };
    Ok(arch)
}

fn package_meta(target: &str) -> anyhow::Result<Package> {
    Ok(Package {
        name: "dorfconf".parse()?,
        version: "0.1.0".parse()?,
        license: "Apache 2.0".parse()?,
        arch: target_to_arch(target)?,
        maintainer: "Freifunk Düsseldorf".parse()?,
        description: "Configures OpenWRT for using the Freifunk Düsseldorf Mesh".into(),
        installed_size: Default::default(),
        provides: Default::default(),
        depends: Default::default(),
        other: Default::default(),
    })
}

fn dummy_signer() -> PackageSigner {
    PackageSigner::generate(None)
}

fn package<'a>(targets: impl IntoIterator<Item = &'a str>, profile: &'a str) -> anyhow::Result<()> {
    let bin_files = targets
        .into_iter()
        .map(|target| {
            (
                target,
                PathBuf::from(format!("target/{target}/{profile}/dorfconf")),
            )
        })
        .collect::<Vec<_>>();

    let missing_targets = bin_files
        .iter()
        .filter_map(|(t, p)| if !p.exists() { Some(*t) } else { None })
        .collect::<Vec<_>>();
    let expected_target_count = missing_targets.len();
    let build_target_count = build(missing_targets, profile).context("building binary")?;
    if build_target_count < expected_target_count {
        return Err(anyhow!("At least one build failed"));
    }

    let signer = dummy_signer();

    for (target, bin) in bin_files {
        eprintln!("Packaging {target}...");

        let dir = TempDir::new(&format!("dorfconf-pkg-{target}")).context("creating tempdir")?;

        let bin_path = dir.path().join("usr/bin");
        create_dir_all(&bin_path).context("creating tmp bin dir")?;
        fs::copy(bin, bin_path.join("dorfconf")).context("copying binary")?;

        let pkg = package_meta(target).context("package metadata")?;
        let out_file = PathBuf::new()
            .join("dist")
            .join(format!("dorfconf_{target}"));
        pkg.write(out_file, dir, &signer).context("writing ipk")?;
    }

    Ok(())
}
