use std::process::ExitCode;
use std::time::{Duration, Instant};

use ferriscraft_progression_pg::PgProgressionRepository;

const FIRST_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(10);
const TOTAL_BUDGET: Duration = Duration::from_secs(120);

async fn connect_and_migrate(url: &str) -> Result<(), String> {
    let repository = PgProgressionRepository::connect(url)
        .await
        .map_err(|err| err.to_string())?;
    repository.migrate().await.map_err(|err| err.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt().init();

    let Ok(url) = std::env::var("PROGRESSION_DATABASE_URL") else {
        tracing::error!("variable PROGRESSION_DATABASE_URL absente");
        return ExitCode::FAILURE;
    };

    let started = Instant::now();
    let mut delay = FIRST_DELAY;
    let mut attempt = 1_u32;
    loop {
        match connect_and_migrate(&url).await {
            Ok(()) => {
                tracing::info!(attempt, "migrations de progression appliquées");
                return ExitCode::SUCCESS;
            }
            Err(err) => {
                if started.elapsed() + delay > TOTAL_BUDGET {
                    tracing::error!(attempt, %err, "échec des migrations, abandon");
                    return ExitCode::FAILURE;
                }
                tracing::warn!(attempt, %err, retry_in = ?delay, "base indisponible, nouvelle tentative");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_DELAY);
                attempt += 1;
            }
        }
    }
}
