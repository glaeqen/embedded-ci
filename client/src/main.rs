use embedded_ci_client::{get_targets, run_job};
use log::*;
use std::fs;

mod cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let cli = cli::cli();

    let target_info = get_targets(cli.server.clone(), cli.auth_token.clone()).await?;

    info!("Target info:\n{:?}", target_info);

    let elf_file = fs::read(cli.elf_file)?;

    let job_result = run_job(
        cli.server,
        cli.auth_token,
        cli.run_on,
        cli.timeout,
        &elf_file,
    )
    .await?;

    println!("{}", job_result);

    Ok(())
}
