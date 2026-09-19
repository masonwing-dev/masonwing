use masonwing_contracts::ScaffoldStatus;
use sqlx::postgres::PgPoolOptions;
use std::{path::Path, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
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
        Some("db") if arguments.get(1).is_some_and(|v| v == "migrate") => {
            if arguments
                .iter()
                .skip(2)
                .any(|v| !matches!(v.as_str(), "--local" | "--adopt-bootstrap"))
            {
                return Err("usage: masonwing db migrate [--local] [--adopt-bootstrap]".into());
            }
            let local = arguments.iter().any(|v| v == "--local");
            let database_url = if local {
                "postgresql://masonwing_migrator:masonwing-local-migrator@127.0.0.1:39852/masonwing"
                    .to_owned()
            } else {
                std::env::var("MASONWING_MIGRATOR_DATABASE_URL")
                    .map_err(|_| "MIGRATOR_DATABASE_URL_REQUIRED")?
            };
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&database_url)
                .await
                .map_err(|_| "MIGRATOR_DATABASE_UNAVAILABLE")?;
            let directory = Path::new("infra/migrations");
            if arguments.iter().any(|v| v == "--adopt-bootstrap") {
                masonwing_data_postgres::migrations::adopt_bootstrap(&pool, directory).await?;
                println!("Adopted existing 0000/0001 bootstrap schema into SQLx history.");
            }
            masonwing_data_postgres::migrations::migrate(&pool, directory).await?;
            println!("Database migrations applied; SQLx checksum drift check passed.");
            pool.close().await;
            Ok(())
        }
        _ => {
            Err("usage: masonwing <status|version|db migrate [--local] [--adopt-bootstrap]>".into())
        }
    }
}
