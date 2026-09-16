use masonwing_contracts::ScaffoldStatus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("status") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&ScaffoldStatus::local("masonwing-cli"))?
            );
            Ok(())
        }
        Some("version") => {
            println!("masonwing {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => Err("usage: masonwing <status|version>; product commands are NOT_IMPLEMENTED".into()),
    }
}
