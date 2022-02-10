#![warn(missing_docs)]

//! Here you can find helper functions for implementing runners that talk to the `embedded-ci`.

use anyhow::anyhow;
pub use embedded_ci_server::*;
use log::*;
use reqwest::StatusCode;
pub use reqwest::{Error, Url};
use std::time::Duration;
use tokio::time;

/// Error definitions for the client calls.
#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    /// A request error.
    #[error("A request failed")]
    Request(#[from] reqwest::Error),
    /// Unauthorized.
    #[error("Unauthorized: Token authentication failed")]
    Unauthorized,
    /// Generic error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Get the available targets from the server.
pub async fn get_targets(server: Url, auth_token: Option<String>) -> Result<Targets, ClientError> {
    let client = reqwest::Client::new().get(server.clone());
    let client = if let Some(auth_token) = &auth_token {
        client.bearer_auth(auth_token.clone())
    } else {
        client
    };

    let response = client.send().await?;

    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(ClientError::Unauthorized);
    }

    Ok(response.json().await?)
}

/// Run a job on the server.
///
/// This will return the log from the target if successful.
pub async fn run_job(
    server: Url,
    auth_token: Option<String>,
    run_on: RunOn,
    timeout: Duration,
    elf_file: &[u8],
) -> Result<String, ClientError> {
    let run_test = RunJob {
        run_on: run_on,
        binary_b64: base64::encode(elf_file),
        timeout_secs: timeout.as_secs() as u32,
    };

    let client = reqwest::Client::new().post(server.join("/run_job").unwrap());
    let client = if let Some(auth_token) = &auth_token {
        client.bearer_auth(auth_token.clone())
    } else {
        client
    };
    let response = client.json(&run_test).send().await?;

    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(ClientError::Unauthorized);
    }

    let res: Result<u32, String> = response.json().await?;

    match res {
        Ok(val) => loop {
            let client =
                reqwest::Client::new().get(server.join(&format!("/status/{}", val)).unwrap());
            let client = if let Some(auth_token) = &auth_token {
                client.bearer_auth(auth_token.clone())
            } else {
                client
            };

            let body: Result<JobStatus, String> = client.send().await?.json().await?;

            match body {
                Ok(status) => match status {
                    JobStatus::WaitingInQueue => info!("{}: Waiting in queue...", val),
                    JobStatus::Running => info!("{}: Running...", val),
                    JobStatus::Done { log } => {
                        info!("{}: Finished successfully with log:", val);
                        return Ok(log);
                    }
                    JobStatus::Error(err) => {
                        return Err(anyhow!("{}: Finished with error: {}", val, err))?;
                    }
                },
                Err(err) => {
                    return Err(anyhow!("{}: Request failure with error: {}", val, err))?;
                }
            }

            time::sleep(Duration::from_secs(1)).await;
        },
        Err(err) => {
            return Err(anyhow!("Error in request to CI: {}", err))?;
        }
    }
}
