use std::{
    borrow::Borrow,
    fs::{self, create_dir_all},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

use anyhow::{Context, anyhow};
use clap::{Parser, Subcommand};
use tempdir::TempDir;
use wolfpack::ipk::{Package, PackageSigner};

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

const DEFAULT_ARCHS: &[&str] = &["mipsel_24kc"];

struct ArchInfo {
    name: &'static str,
    rust_target: &'static str,
}

fn instruction_set_info(instruction_set: &str) -> ArchInfo {
    // this list is incomplete - some architectures are not supported
    // for a full list, see
    // https://openwrt.org/docs/techref/instructionset/start#all_instruction_sets
    match instruction_set {
        "mipsel_24kc" => ArchInfo {
            name: "mipsel_24kc",
            rust_target: "mipsel-unknown-linux-musl",
        },
        _ => unimplemented!(
            "Instruction set {} is currently not supported",
            instruction_set
        ),
    }
}

fn main() -> anyhow::Result<()> {
    let input = Cli::parse();

    // todo: take override from command line arg
    let archs = DEFAULT_ARCHS.into_iter().map(|a| instruction_set_info(*a));
    let profile = "debug";

    match input.command {
        Command::Build => {
            let target_num = build(archs, profile)?;
            if target_num == 0 {
                eprintln!("Warning: No targets built!")
            }
        }
        Command::Package => package(archs, profile)?,
    }

    Ok(())
}

fn build<'a, Arch: Borrow<ArchInfo>>(
    archs: impl IntoIterator<Item = Arch>,
    profile: &'a str,
) -> anyhow::Result<usize> {
    let cross_installed = which::which("cross").is_ok();
    if !cross_installed {
        return Err(anyhow!(
            "Please install Cross first: https://crates.io/crates/cross"
        ));
    }

    let handles = archs
        .into_iter()
        .map(|a| {
            duct::cmd(
                "cross",
                &[
                    "build",
                    "--package",
                    "dorfconf",
                    "--profile",
                    profile,
                    "--target",
                    a.borrow().rust_target,
                ],
            )
            .stdout_to_stderr()
            .stderr_capture()
            .unchecked()
            .start()
            .map(|h| (a, h))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let target_num = handles.len();
    for (arch, handle) in handles {
        eprintln!("Arch: {}", arch.borrow().name);
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

fn package_meta(arch: &str) -> anyhow::Result<Package> {
    let pkg = Package {
        name: "dorfconf".parse()?,
        version: "0.1.0".parse()?,
        license: "Apache 2.0".parse()?,
        arch: "noarch".parse()?, // overridden below
        maintainer: "Freifunk Düsseldorf".parse()?,
        description: "Configures OpenWRT for using the Freifunk Düsseldorf Mesh".into(),
        installed_size: Default::default(),
        provides: Default::default(),
        depends: Default::default(),
        // hack to support architectures not in the wolfpack enum
        other: format!("Architecture: {arch}").parse()?,
    };
    Ok(pkg)
}

fn dummy_signer() -> PackageSigner {
    PackageSigner::generate(None)
}

fn package<'a>(archs: impl IntoIterator<Item = ArchInfo>, profile: &'a str) -> anyhow::Result<()> {
    let bin_files = archs
        .into_iter()
        .map(|arch| {
            let path = format!("target/{}/{}/dorfconf", arch.rust_target, profile);
            (arch, PathBuf::from(path))
        })
        .collect::<Vec<_>>();

    let missing_targets = bin_files
        .iter()
        .filter_map(|(t, p)| if !p.exists() { Some(t) } else { None })
        .collect::<Vec<_>>();
    let expected_target_count = missing_targets.len();
    let build_target_count = build(missing_targets, profile).context("building binary")?;
    if build_target_count < expected_target_count {
        return Err(anyhow!("At least one build failed"));
    }

    let signer = dummy_signer();

    for (arch, bin) in bin_files {
        eprintln!("Packaging {}...", arch.name);

        let dir =
            TempDir::new(&format!("dorfconf-pkg-{}", arch.name)).context("creating tempdir")?;

        let bin_path = dir.path().join("usr/bin");
        create_dir_all(&bin_path).context("creating tmp bin dir")?;
        fs::copy(bin, bin_path.join("dorfconf")).context("copying binary")?;

        {
            let defaults_dir = dir.path().join("etc").join("uci-defaults");
            create_dir_all(&defaults_dir)?;
            let mut uci_default_script = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o755)
                .open(defaults_dir.join("01_ffddorf_config"))?;
            write!(&mut uci_default_script, "#!/bin/sh\nexec dorfconfig")?;
        }

        let pkg = package_meta(arch.name).context("package metadata")?;
        let out_file = PathBuf::new()
            .join("dist")
            .join(format!("dorfconf_{}.ipk", arch.name));
        pkg.write(out_file, dir, &signer).context("writing ipk")?;
    }

    Ok(())
}
