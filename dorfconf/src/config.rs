use std::{collections::HashMap, net, os::unix::process::ExitStatusExt, process::Command};

use anyhow::anyhow;
use facet::Facet;
use rust_uci::config::Config as UciConfig;

#[derive(Clone, Facet)]
pub struct TunnelPeer {
    host: String,
    port: u16,
    pubkey: String,
}

#[derive(Clone, Facet)]
pub struct DomainConfig {
    zip_codes: Vec<String>,
    fastd_peers: Vec<TunnelPeer>,
}

#[derive(Facet)]
pub struct ManagedConfig {
    domains: HashMap<String, DomainConfig>,
}

impl ManagedConfig {
    pub fn select_domain<'a>(&'a self, zip_code: impl AsRef<str>) -> Option<&'a DomainConfig> {
        for (_, conf) in &self.domains {
            if let Some(_) = conf
                .zip_codes
                .iter()
                .find(|code| *code == zip_code.as_ref())
            {
                return Some(conf);
            }
        }
        None
    }
}

pub struct Configurator {
    uci: UciConfig,
    config: ManagedConfig,
}

impl Configurator {
    pub fn new(uci: UciConfig, config: ManagedConfig) -> Self {
        Self { uci, config }
    }

    fn find_domain(&self) -> anyhow::Result<&DomainConfig> {
        let ffddorf = self
            .uci
            .package("ffddorf")?
            .ok_or_else(|| anyhow!("config package for ffddorf not found"))?;
        let node_config = ffddorf
            .sections_by_type("node")?
            .next()
            .ok_or_else(|| anyhow!("node config not found"))?;

        let zip_code = node_config.option("zip_code")?.get()?;
        let zip_code = zip_code.as_ref().and_then(|v| v.to_str());

        let domain_id = node_config.option("domain")?.get()?;
        let domain_id = domain_id.as_ref().and_then(|v| v.to_str());

        let config = match (zip_code, domain_id) {
            (_, Some(domain_id)) => self
                .config
                .domains
                .get(domain_id)
                .ok_or_else(|| anyhow!("domain for ID {} not found", domain_id))?,
            (Some(zip_code), _) => self
                .config
                .select_domain(zip_code)
                .ok_or_else(|| anyhow!("domain for zip code {} not found", zip_code))?,
            _ => {
                return Err(anyhow!(
                    "neither domain ID nor zip code specified (lists are ignored)"
                ));
            }
        };
        Ok(config)
    }

    pub fn setup_all(&self) -> anyhow::Result<()> {
        let domain = self.find_domain()?;

        self.setup_batman()?;
        self.setup_lan()?;
        self.setup_fastd(domain)?;
        self.setup_wifi()?;

        Ok(())
    }

    pub fn setup_batman(&self) -> anyhow::Result<()> {
        let mut network = self
            .uci
            .package("network")?
            .ok_or_else(|| anyhow!("Package network missing"))?;

        // create interface
        let mut bat0 = network.section("interface", "bat0")?;
        bat0.option_mut("proto")?.set("batadv")?;
        bat0.option_mut("routing_algo")?.set("BATMAN_IV")?;
        bat0.option_mut("gw_mode")?.set("client")?;

        // add to lan bridge
        let bridge_name = network
            .section("interface", "lan")?
            .option("device")?
            .get()?
            .ok_or_else(|| anyhow!("LAN bridge not found"))?;
        let mut bridge_dev = network
            .sections_by_type("device")?
            .filter_map(|s| {
                s.option("name")
                    .ok()?
                    .get()
                    .ok()?
                    .filter(|val| val == &bridge_name)
                    .map(|_| s)
            })
            .next()
            .ok_or_else(|| {
                anyhow!(r#"Unable to find device for LAN bridge {bridge_name:?} in config"#)
            })?;
        bridge_dev.option_mut("ports")?.add_list("bat0")?;

        network.save()?;
        network.commit()?;
        Ok(())
    }

    // Configure lan bridge interface to be a regular client in the mesh
    pub fn setup_lan(&self) -> anyhow::Result<()> {
        let mut network = self
            .uci
            .package("network")?
            .ok_or_else(|| anyhow!("Package network missing"))?;

        // disable the default lan config
        let mut lan = network.section("interface", "lan")?;
        lan.option_mut("proto")?.set("none")?;

        // add a special v6only client config
        let bridge_name_opt = lan
            .option("device")?
            .get()?
            .ok_or_else(|| anyhow!("LAN bridge not found"))?;
        let bridge_name = bridge_name_opt
            .to_str()
            .ok_or_else(|| anyhow!("Invalid value for bridge_name"))?;
        let mut node6 = network.section("interface", "node6")?;
        node6.option_mut("device")?.set(&bridge_name)?;
        node6.option_mut("proto")?.set("dhcpv6")?;
        node6.option_mut("accept_ra")?.set("1")?;
        node6.option_mut("force_link")?.set("1")?;
        node6.option_mut("bridge_empty")?.set("1")?;
        node6.option_mut("defaultroute")?.set("0")?;
        node6.option_mut("reqprefix")?.set("no")?;

        network.save()?;
        network.commit()?;
        Ok(())
    }

    fn fastd_gen_key() -> anyhow::Result<String> {
        let gen_out = Command::new("fastd")
            .args(["--generate-key", "--machine-readable"])
            .output()?;
        if !gen_out.status.success() {
            let stderr = String::from_utf8_lossy(&gen_out.stderr);
            return match (gen_out.status.code(), gen_out.status.signal()) {
                (_, Some(signal)) => Err(anyhow!(
                    "Failed to run fastd. Terminated with signal {signal}\n{stderr}",
                )),
                (Some(code), _) => Err(anyhow!("Failed to run fastd. Exit {code}\n{stderr}")),
                _ => Err(anyhow!("Failed to run fastd.\n{stderr}")),
            };
        }

        Ok(String::from_utf8(gen_out.stdout)?.trim().into())
    }

    pub fn setup_fastd(&self, config: &DomainConfig) -> anyhow::Result<()> {
        // basic settings
        let fastd_secret_key = Self::fastd_gen_key()?;
        let mut fastd = self.uci.package("fastd")?.ok_or_else(|| {
            anyhow!("fastd config package not found, consider installing the package")
        })?;
        let mut meshvpn = fastd.section("fastd", "meshvpn")?;
        meshvpn.option_mut("enabled")?.set("1")?;
        meshvpn.option_mut("syslog_level")?.set("info")?;
        meshvpn.option_mut("method")?.set("null@l2tp")?;
        meshvpn.option_mut("offload_l2tp")?.set("1")?;
        meshvpn.option_mut("mtu")?.set("1364")?;
        meshvpn.option_mut("interface")?.set("mesh-vpn")?;
        meshvpn.option_mut("secret")?.set(&fastd_secret_key)?;
        meshvpn.option_mut("forward")?.set("0")?;
        meshvpn.option_mut("persist_interface")?.set("0")?;
        meshvpn
            .option_mut("on_up")?
            .set("ip link set dev $INTERFACE master bat0 && ip link set dev $INTERFACE up")?;

        for (i, peer) in config.fastd_peers.iter().enumerate() {
            let mut supernode = fastd.section("peer", format!("supernode{i}"))?;
            supernode.option_mut("enabled")?.set("1")?;
            supernode.option_mut("net")?.set("meshvpn")?;
            supernode.option_mut("key")?.set(&peer.pubkey)?;

            let addr = peer
                .host
                .parse::<net::IpAddr>()
                .ok()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| format!(r#""{}""#, peer.host)); // quote when not an IP
            supernode
                .option_mut("remote")?
                .set(format!("{} port {}", addr, peer.port))?;
        }

        fastd.save()?;
        fastd.commit()?;
        Ok(())
    }

    pub fn setup_wifi(&self) -> anyhow::Result<()> {
        let mut wireless = self
            .uci
            .package("wireless")?
            .ok_or_else(|| anyhow!("wireless config package not found"))?;

        // find non-2ghz wifi interfaces
        let devices = wireless.sections_by_type("wifi-device")?.filter_map(|s| {
            s.option("band")
                .ok()?
                .get()
                .ok()??
                .to_str()
                .filter(|band| *band != "2g")
                .map(|_| s)
        });

        for (i, mut dev) in devices.enumerate() {
            let Some(dev_name) = dev.name() else {
                continue;
            };

            dev.option_mut("disabled")?.set("0")?;

            // configure freifunk SSID
            let mut ffap = dev
                .package()
                .section("wifi-iface", format!("freifunk{i}"))?;
            ffap.option_mut("device")?.set(dev_name)?;
            ffap.option_mut("network")?.set("lan")?;
            ffap.option_mut("mode")?.set("ap")?;
            ffap.option_mut("ssid")?.set("Freifunk Dev")?;
            ffap.option_mut("encryption")?.set("none")?;
        }

        wireless.save()?;
        wireless.commit()?;
        Ok(())
    }
}
