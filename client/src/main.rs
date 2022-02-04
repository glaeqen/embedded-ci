use anyhow::anyhow;
use log::*;
use std::{fs, time::Duration};
use tokio::time;

mod cli;
mod requests;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let cli = cli::cli();

    let client = reqwest::Client::new().get(cli.server.clone());
    let client = if let Some(auth_token) = &cli.auth_token {
        client.bearer_auth(auth_token.clone())
    } else {
        client
    };
    let body: requests::Targets = client.send().await?.json().await?;

    info!("Target info:\n{:?}", body);
    // println!("'/': {}", body);

    let elf_file = fs::read(cli.elf_file)?;
    let run_test = requests::RunJob {
        run_on: cli.run_on,
        binary_b64: base64::encode(elf_file),
        timeout_secs: 30,
    };

    let client = reqwest::Client::new().post(cli.server.join("/run_job").unwrap());
    let client = if let Some(auth_token) = &cli.auth_token {
        client.bearer_auth(auth_token.clone())
    } else {
        client
    };
    let res: Result<u32, String> = client.json(&run_test).send().await?.json().await?;

    match res {
        Ok(val) => loop {
            let client =
                reqwest::Client::new().get(cli.server.join(&format!("/status/{}", val)).unwrap());
            let client = if let Some(auth_token) = &cli.auth_token {
                client.bearer_auth(auth_token.clone())
            } else {
                client
            };

            let body: Result<requests::JobStatus, String> = client.send().await?.json().await?;

            match body {
                Ok(status) => match status {
                    requests::JobStatus::WaitingInQueue => info!("{}: Waiting in queue...", val),
                    requests::JobStatus::Running => info!("{}: Running...", val),
                    requests::JobStatus::Done { log } => {
                        info!("{}: Finished successfully with log:", val);
                        println!("{}", log);
                        break;
                    }
                    requests::JobStatus::Error(err) => {
                        return Err(anyhow!("{}: Finished with error: {}", val, err));
                    }
                },
                Err(err) => {
                    return Err(anyhow!("{}: Request failure with error: {}", val, err));
                }
            }

            time::sleep(Duration::from_secs(1)).await;
        },
        Err(err) => {
            return Err(anyhow!("Error in request to CI: {}", err));
        }
    }

    Ok(())
}
