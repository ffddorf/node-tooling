use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};

use anyhow::{Context, anyhow};
use tempdir::TempDir;
use wolfpack::ipk::{Package, PackageSigner};

use crate::{OpenWrtArch, Profile, build::build};

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

pub fn package<'a>(
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
        fs::create_dir_all(&bin_path).context("creating tmp bin dir")?;
        fs::copy(bin, bin_path.join("dorfconf")).context("copying binary")?;

        {
            let defaults_dir = dir.path().join("etc").join("uci-defaults");
            fs::create_dir_all(&defaults_dir)?;
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
