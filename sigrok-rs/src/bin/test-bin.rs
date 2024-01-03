use std::{io::Write, time::Duration};

use sigrok_rs::LogicAnalyzer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    for mut la in LogicAnalyzer::all()? {
        let active_capture = la.start_capture(1).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let capture = active_capture.stop_capture().await?;
        std::io::stdout().write_all(&capture)?;
    }
    Ok(())
}
