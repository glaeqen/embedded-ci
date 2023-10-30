use log::*;
use std::sync::{Arc, Mutex};
use tokio::signal;

mod app;
mod auth;
mod cli;
mod routes;
mod runner;
mod target;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let cli = match cli::cli() {
        Ok(v) => v,
        Err(e) => {
            println!("Error in startup: {}", e);
            std::process::exit(1);
        }
    };

    auth::set_token(cli.auth_tokens);

    let targets = match cli::from_cli(&cli.probe_configs) {
        Ok(v) => v,
        Err(e) => {
            println!("Error in startup: {}", e);
            std::process::exit(1);
        }
    };

    debug!("Targets: {:#?}", targets);

    let jobs = Arc::new(Mutex::new(app::RunQueue::new(
        targets,
        cli.server_configs.max_jobs_in_queue.0,
    )));

    let rocket_jobs = jobs.clone();
    let _rocket_handle = tokio::spawn(async move { routes::serve_routes(rocket_jobs).await });

    let backend_jobs = jobs.clone();
    let probe_mutex = Arc::new(Mutex::new(()));
    let _backend_handle = tokio::spawn(async move {
        app::Backend::run(
            backend_jobs,
            cli.probe_configs,
            cli.server_configs,
            probe_mutex,
        )
        .await
    });

    let cleanup_jobs = jobs.clone();
    let _cleanup_handle = tokio::spawn(async move { app::Cleanup::run(cleanup_jobs).await });

    match signal::ctrl_c().await {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
        }
    }

    Ok(())
}
