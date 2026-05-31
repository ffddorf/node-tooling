//! This crate provides a tool to run in OpenWRT `uci-defaults` to setup
//! a node in the mesh network of Freifunk Düsseldorf.

use std::{env, fs::File, io::Read};

use anyhow::Context;
use rust_uci::Uci;

use crate::config::Configurator;

mod config;

fn main() -> anyhow::Result<()> {
    let uci = Uci::new()?;

    let config_path = match env::var("DORFCONF_CONFIG_PATH") {
        Ok(path) => path,
        Err(env::VarError::NotPresent) => "/etc/ffddorf/managed.json".into(),
        Err(e) => return Err(e.into()),
    };

    let config = {
        let mut file = File::open(&config_path)
            .with_context(|| format!("open config file at {}", config_path))?;
        let mut json = Vec::new();
        file.read_to_end(&mut json)?;
        facet_json::from_slice(&json).context("parse config from json")?
    };

    let runner = Configurator::new(uci.into(), config);
    runner.setup_all()?;
    Ok(())
}
