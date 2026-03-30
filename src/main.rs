use rust_uci::Uci;

fn main() -> anyhow::Result<()> {
    let mut uci = Uci::new()?;

    let hostname = uci.get("system.hostname")?;
    println!("Hostname from UCI: {hostname}");

    let net = uci.get_sections("network")?;
    println!("Network sections: {net:?}");

    Ok(())
}
