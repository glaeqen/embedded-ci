#![warn(missing_docs)]

//! Embedded CI server

use log::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::signal;

use embedded_ci_common::ServerStatus;

mod app;
mod auth;
mod cli;
mod routes;
mod runner;

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
    let max_jobs_in_queue = cli.server_configs.max_jobs_in_queue.0;

    let (register_job_tx, register_job_rx) = tokio::sync::mpsc::channel(max_jobs_in_queue);

    let (finished_job_tx, finished_job_rx) = tokio::sync::mpsc::channel(max_jobs_in_queue);

    let server_status = Arc::new(Mutex::new(ServerStatus::default()));

    let finished_job_queue = Arc::new(Mutex::new(VecDeque::with_capacity(max_jobs_in_queue)));

    let _rocket_handle = tokio::spawn(routes::serve(
        finished_job_queue.clone(),
        register_job_tx,
        targets,
        server_status.clone(),
    ));

    let _finished_job_collector = tokio::spawn(app::finished_job_collector(
        finished_job_queue.clone(),
        finished_job_rx,
        server_status.clone(),
        max_jobs_in_queue,
    ));

    let _backend_handle = tokio::spawn(app::run(
        register_job_rx,
        finished_job_tx,
        server_status.clone(),
        cli.probe_configs,
        cli.server_configs,
    ));

    match signal::ctrl_c().await {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
        }
    }

    Ok(())
}
