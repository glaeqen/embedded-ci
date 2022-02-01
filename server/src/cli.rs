use anyhow::anyhow;
use clap::Parser;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use schemars::JsonSchema;
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

/// Probe serial wrapper.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct ProbeSerial(pub String);

/// Name of an authorization token wrapper.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct AuthName(pub String);

/// Authorization token wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthToken(pub String);

/// Information about a probe, used for storing and reading configurations.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeInfo {
    pub target_name: String,
    #[serde(default)]
    pub probe_alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_speed_khz: Option<u32>,
}

pub struct Cli {
    pub probe_configs: HashMap<ProbeSerial, ProbeInfo>,
    pub auth_tokens: HashMap<AuthName, AuthToken>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_tokens: Option<HashMap<AuthName, AuthToken>>,
    probe_configs: HashMap<ProbeSerial, ProbeInfo>,
}

impl SavedSettings {
    fn validate(&self) -> anyhow::Result<()> {
        if let Some(tokens) = &self.auth_tokens {
            for (k, v) in tokens {
                if k.0.is_empty() {
                    return Err(anyhow!(
                        "Invalid authorization tokens detected, name is not filled"
                    ));
                }

                if v.0.is_empty() {
                    return Err(anyhow!(
                        "Invalid authorization tokens detected, value is not filled"
                    ));
                }
            }
        }

        for (k, v) in &self.probe_configs {
            if k.0.is_empty() {
                return Err(anyhow!(
                    "Invalid probe config detected, probe serial is not filled"
                ));
            }

            if v.target_name.is_empty() {
                return Err(anyhow!(
                    "Invalid probe config detected, 'target_name' is not filled"
                ));
            }
        }

        Ok(())
    }
}

pub fn cli() -> anyhow::Result<Cli> {
    let args = Args::parse();

    let s = fs::read_to_string(&args.probe_config)?;
    let mut settings: SavedSettings = serde_json::from_str(&s)?;

    settings.validate()?;

    let mut auth_tokens = settings.auth_tokens.unwrap_or_default();

    if let Some(token_name) = args.new_token {
        let random_string: AuthToken = AuthToken(
            thread_rng()
                .sample_iter(&Alphanumeric)
                .take(128)
                .map(char::from)
                .collect(),
        );

        println!("Added new token '{}': {}", token_name, random_string.0);
        auth_tokens.insert(AuthName(token_name), random_string);

        settings.auth_tokens = Some(auth_tokens.clone());

        fs::write(
            &args.probe_config,
            &serde_json::to_string_pretty(&settings)?,
        )?;

        std::process::exit(0);
    }

    Ok(Cli {
        probe_configs: settings.probe_configs,
        auth_tokens,
    })
}
