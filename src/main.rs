//! This crate provides a tool to run in OpenWRT `uci-defaults` to setup
//! a node in the mesh network of Freifunk Düsseldorf.

use core::net;
use std::{os::unix::process::ExitStatusExt, process::Command};

use anyhow::anyhow;
use rust_uci::Uci;

fn main() -> anyhow::Result<()> {
    let uci = Uci::new()?;

    let config = Config {
        fastd_peers: vec![TunnelPeer::new(
            "supernode-dev-0.ffddorf.net",
            10000,
            "ca06ceb16e88061bef81c81ae75eb86b9f387b403fdf3cf7450b6838a2a8f570",
        )],
    };

    let mut conf = Configurator::new(uci, config);
    conf.setup_batman()?;
    conf.setup_lan()?;
    conf.setup_fastd()?;
    conf.setup_wifi()?;

    Ok(())
}

struct TunnelPeer {
    host: String,
    port: u16,
    pubkey: String,
}

impl TunnelPeer {
    pub fn new(host: impl Into<String>, port: u16, pubkey: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            pubkey: pubkey.into(),
        }
    }
}

struct Config {
    fastd_peers: Vec<TunnelPeer>,
}

struct Configurator {
    uci: Uci,
    config: Config,
}

impl Configurator {
    pub fn new(uci: Uci, config: Config) -> Self {
        Self {
            uci: uci.into(),
            config,
        }
    }

    fn find_sections<'a, F>(
        &'a mut self,
        package: &'a str,
        section_type: &'a str,
        predicate: F,
    ) -> anyhow::Result<impl Iterator<Item = String> + 'a>
    where
        F: Fn(&mut Uci, &str) -> anyhow::Result<bool> + 'static,
    {
        let sections = self.uci.get_sections(package)?;
        let logical = sections.into_iter().filter_map(move |section| {
            if self.uci.get(&format!("{package}.{section}")).ok()? == section_type
                && predicate(&mut self.uci, &section).ok()?
            {
                return Some(section);
            }
            None
        });
        Ok(logical)
    }

    pub fn setup_batman(&mut self) -> anyhow::Result<()> {
        // create interface
        self.uci.set("network.bat0", "interface")?;
        self.uci.set("network.bat0.proto", "batadv")?;
        self.uci.set("network.bat0.routing_algo", "BATMAN_IV")?;
        self.uci.set("network.bat0.gw_mode", "client")?;

        // add to lan bridge
        let bridge_name = self.uci.get("network.lan.device")?;

        let Some(bridge_dev) = self
            .find_sections("network", "device", move |uci, dev| {
                Ok(uci.get(&format!("network.{dev}.name"))? == bridge_name)
            })?
            .next()
        else {
            return Err(anyhow!("Unable to find device for LAN bridge in config"));
        };

        self.uci
            .add_list(format!("network.{bridge_dev}.ports"), "bat0")?;

        self.uci.commit("network")?;
        Ok(())
    }

    // Configure lan bridge interface to be a regular client in the mesh
    pub fn setup_lan(&mut self) -> anyhow::Result<()> {
        // disable the default lan config
        self.uci.set("network.lan.proto", "none")?;

        // add a special v6only client config
        let bridge_name = self.uci.get("network.lan.device")?;
        self.uci.set("network.node6", "interface")?;
        self.uci.set("network.node6.device", &bridge_name)?;
        self.uci.set("network.node6.proto", "dhcpv6")?;
        self.uci.set("network.node6.accept_ra", "1")?;
        self.uci.set("network.node6.force_link", "1")?;
        self.uci.set("network.node6.bridge_empty", "1")?;
        self.uci.set("network.node6.defaultroute", "0")?;
        self.uci.set("network.node6.reqprefix", "no")?;

        self.uci.commit("network")?;
        Ok(())
    }

    fn fastd_gen_key() -> anyhow::Result<String> {
        let gen_out = Command::new("fastd")
            .args(["--generate-key", "--machine-readable"])
            .output()?;
        if !gen_out.status.success() {
            return match (gen_out.status.code(), gen_out.status.signal()) {
                (_, Some(signal)) => Err(anyhow!(
                    "Failed to run fastd. Terminated with signal {}",
                    signal
                )),
                (Some(code), _) => Err(anyhow!(
                    "Failed to run fastd. Exit {}\n{}",
                    code,
                    String::from_utf8_lossy(&gen_out.stderr)
                )),
                _ => Err(anyhow!(
                    "Failed to run fastd.\n{}",
                    String::from_utf8_lossy(&gen_out.stderr)
                )),
            };
        }

        Ok(String::from_utf8(gen_out.stdout)?.trim().into())
    }

    pub fn setup_fastd(&mut self) -> anyhow::Result<()> {
        // basic settings
        let fastd_secret_key = Self::fastd_gen_key()?;
        self.uci.set("fastd.meshvpn", "fastd")?;
        self.uci.set("fastd.meshvpn.enabled", "1")?;
        self.uci.set("fastd.meshvpn.syslog_level", "info")?;
        self.uci.set("fastd.meshvpn.method", "null@l2tp")?;
        self.uci.set("fastd.meshvpn.offload_l2tp", "1")?;
        self.uci.set("fastd.meshvpn.mtu", "1364")?;
        self.uci.set("fastd.meshvpn.interface", "mesh-vpn")?;
        self.uci.set("fastd.meshvpn.secret", &fastd_secret_key)?;
        self.uci.set("fastd.meshvpn.forward", "0")?;
        self.uci.set("fastd.meshvpn.persist_interface", "0")?;
        self.uci.set(
            "fastd.meshvpn.on_up",
            "ip link set dev $INTERFACE master bat0 && ip link set dev $INTERFACE up",
        )?;

        for (i, peer) in self.config.fastd_peers.iter().enumerate() {
            let section = format!("fastd.supernode{i}");
            self.uci.set(&section, "peer")?;
            self.uci.set(&format!("{section}.enabled"), "1")?;
            self.uci.set(&format!("{section}.net"), "meshvpn")?;
            self.uci.set(&format!("{section}.key"), &peer.pubkey)?;
            let addr = peer
                .host
                .parse::<net::IpAddr>()
                .ok()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| format!(r#""{}""#, peer.host)); // quote when not an IP
            self.uci.set(
                &format!("{section}.remote"),
                &format!("{} port {}", addr, peer.port),
            )?;
        }

        self.uci.commit("fastd")?;
        Ok(())
    }

    pub fn setup_wifi(&mut self) -> anyhow::Result<()> {
        // find non-2ghz wifi interfaces
        let devices: Vec<_> = self
            .find_sections("wireless", "wifi-device", |uci, dev| {
                Ok(uci.get(&format!("wireless.{dev}.band"))? == "2g")
            })?
            .collect();

        for (i, dev) in devices.iter().enumerate() {
            // enable device
            self.uci.set(&format!("wireless.{dev}.disabled"), "0")?;

            // configure freifunk SSID
            self.uci
                .set(&format!("wireless.freifunk{i}.device"), "radio1")?;
            self.uci
                .set(&format!("wireless.freifunk{i}.network"), "lan")?;
            self.uci.set(&format!("wireless.freifunk{i}.mode"), "ap")?;
            self.uci
                .set(&format!("wireless.freifunk{i}.ssid"), "Freifunk Dev")?;
            self.uci
                .set(&format!("wireless.freifunk{i}.encryption"), "none")?;
        }

        self.uci.commit("wireless")?;
        Ok(())
    }
}
