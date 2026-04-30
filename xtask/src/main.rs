use std::{
    error::Error,
    fmt::Display,
    fs::{self, create_dir_all},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    str::FromStr,
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

const DEFAULT_ARCHS: &[OpenWrtArch] = &[OpenWrtArch::Mipsel24kc];

#[non_exhaustive]
#[derive(Copy, Clone)]
enum OpenWrtArch {
    Mipsel24kc,
}

impl OpenWrtArch {
    pub fn rust_target(&self) -> &'static str {
        match self {
            OpenWrtArch::Mipsel24kc => "mipsel-unknown-linux-musl",
        }
    }
}

impl Display for OpenWrtArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenWrtArch::Mipsel24kc => f.write_str("mipsel_24kc"),
        }
    }
}

#[derive(Debug)]
struct UnsupportedArch(String);

impl Error for UnsupportedArch {}

impl Display for UnsupportedArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Architecture not supported: {}", self.0)
    }
}

impl FromStr for OpenWrtArch {
    type Err = UnsupportedArch;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mipsel_24kc" => Ok(Self::Mipsel24kc),
            _ => Err(UnsupportedArch(s.to_owned())),
        }
    }
}

fn main() -> anyhow::Result<()> {
    let input = Cli::parse();

    // todo: take override from command line arg
    let archs = DEFAULT_ARCHS;
    let profile = Profile::Debug;

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

enum Profile {
    Debug,
    Release,
    Custom(String),
}

impl Profile {
    pub fn target_subdir(&self) -> &str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
            Profile::Custom(custom) => custom,
        }
    }

    pub fn cargo_arg(&self) -> Box<[&str]> {
        match self {
            Profile::Debug => [].into(),
            Profile::Release => ["--release"].into(),
            Profile::Custom(custom) => ["--profile", custom].into(),
        }
    }
}

fn build<'a>(
    archs: impl IntoIterator<Item = OpenWrtArch>,
    profile: Profile,
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
            let mut args = vec![
                "build",
                "--package",
                "dorfconf",
                "--target",
                a.rust_target(),
            ];
            args.extend_from_slice(&profile.cargo_arg());
            duct::cmd("cross", &args)
                .stdout_to_stderr()
                .stderr_capture()
                .unchecked()
                .start()
                .map(|h| (a, h))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let target_num = handles.len();
    for (arch, handle) in handles {
        eprintln!("Arch: {arch}");
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

fn package_meta(arch: OpenWrtArch) -> anyhow::Result<Package> {
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

fn package<'a>(
    archs: impl IntoIterator<Item = OpenWrtArch>,
    profile: Profile,
) -> anyhow::Result<()> {
    let bin_files = archs
        .into_iter()
        .map(|arch| {
            let path = format!(
                "target/{}/{}/dorfconf",
                arch.rust_target(),
                profile.target_subdir()
            );
            (arch, PathBuf::from(path))
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

    for (arch, bin) in bin_files {
        eprintln!("Packaging {}...", arch);

        let dir = TempDir::new(&format!("dorfconf-pkg-{}", arch)).context("creating tempdir")?;

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

        let pkg = package_meta(arch).context("package metadata")?;
        let out_file = PathBuf::new()
            .join("dist")
            .join(format!("dorfconf_{}.ipk", arch));
        pkg.write(out_file, dir, &signer).context("writing ipk")?;
    }

    Ok(())
}
