use log::*;
use std::sync::{Arc, Mutex};
use tokio::signal;

mod app;
mod auth;
mod cli;
mod routes;
mod runner;
mod target;

use target::Targets;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let cli = cli::cli();

    auth::set_token(cli.auth_tokens);

    let targets = Targets::from_target_settings(&cli.probe_configs);

    info!("Targets: {:#?}", targets);

    let jobs = Arc::new(Mutex::new(app::RunQueue::new(targets)));

    let rocket_jobs = jobs.clone();
    let _rocket_handle = tokio::spawn(async move { routes::serve_routes(rocket_jobs).await });

    let backend_jobs = jobs.clone();
    let _backend_handle = tokio::spawn(async move { app::Backend::run(backend_jobs).await });

    match signal::ctrl_c().await {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
        }
    }

    Ok(())
}
