//! ローカルNIC検出の動作確認（FR-01）。root 不要。
//! 実行: `cargo run -p prowl-core --example detect`

fn main() -> anyhow::Result<()> {
    let local = prowl_core::net::detect()?;
    println!("interface : {}", local.interface.name);
    println!("ipv4      : {}", local.ipv4);
    println!("mac       : {}", local.mac);
    println!("subnet    : {}", local.subnet().cidr);

    let targets = local.targets(4096);
    let head = &targets[..targets.len().min(3)];
    println!("targets   : {} 件 (先頭例: {head:?})", targets.len());
    Ok(())
}
