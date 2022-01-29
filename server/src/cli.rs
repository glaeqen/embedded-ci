use crate::target::TargetSettings;
use clap::Parser;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Probe config
    #[clap(short, long)]
    probe_config: PathBuf,

    /// Create a new authorization token with the following name
    #[clap(long)]
    new_token: Option<String>,
}

pub struct Cli {
    pub probe_configs: Vec<TargetSettings>,
    pub auth_tokens: HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_tokens: Option<HashMap<String, String>>,
    probe_configs: Vec<TargetSettings>,
}

pub fn cli() -> Cli {
    let args = Args::parse();

    let s = fs::read_to_string(&args.probe_config).expect("Could not read file with probe configs");
    let mut settings: SavedSettings =
        serde_json::from_str(&s).expect("Error in reading probe configs");

    let mut auth_tokens = settings.auth_tokens.unwrap_or_default();

    if let Some(token_name) = args.new_token {
        let random_string: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(128)
            .map(char::from)
            .collect();

        println!("Added new token '{}': {}", token_name, random_string);
        auth_tokens.insert(token_name, random_string);

        settings.auth_tokens = Some(auth_tokens.clone());

        fs::write(
            &args.probe_config,
            &serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();

        std::process::exit(0);
    }

    Cli {
        probe_configs: settings.probe_configs,
        auth_tokens,
    }
}
