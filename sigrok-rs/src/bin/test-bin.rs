use sigrok_rs::LogicAnalyzer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    for la in LogicAnalyzer::all()? {
        let capture = la.capture_bzipped(10, 1).await?;
        println!("{capture:#0X?}");
    }
    Ok(())
}
