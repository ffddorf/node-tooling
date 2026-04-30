use anyhow::anyhow;

use crate::{OpenWrtArch, Profile};

pub fn build<'a>(
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
