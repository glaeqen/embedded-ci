use std::{fs, time::Duration};

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
    let body = client.send().await?.text().await?;

    println!("'/': {}", body);

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

    // println!("'/run_test': {}", run);

    match res {
        Ok(val) => loop {
            let client =
                reqwest::Client::new().get(cli.server.join(&format!("/status/{}", val)).unwrap());
            let client = if let Some(auth_token) = &cli.auth_token {
                client.bearer_auth(auth_token.clone())
            } else {
                client
            };
            // replace .text() with .json()
            let body = client.send().await?.text().await?;

            println!("Job {} status: {}", val, body);

            std::thread::sleep(Duration::from_secs(1));
        },
        Err(err) => {}
    }

    Ok(())
}
