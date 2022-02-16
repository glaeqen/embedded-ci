use anyhow::anyhow;
use clap::Parser;
use embedded_ci_server::{
    AuthName, AuthToken, ProbeAlias, ProbeSerial, Target, TargetName, Targets,
};
use log::*;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::target::get_mcus;

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

/// Information about a probe, used for storing and reading configurations.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeInfo {
    pub target_name: TargetName,
    #[serde(default)]
    pub probe_alias: ProbeAlias,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_speed_khz: Option<u32>,
}

pub struct Cli {
    pub probe_configs: HashMap<ProbeSerial, ProbeInfo>,
    pub auth_tokens: HashMap<AuthName, AuthToken>,
    pub server_configs: ServerConfigs,
}

pub fn from_cli(target_settings: &HashMap<ProbeSerial, ProbeInfo>) -> anyhow::Result<Targets> {
    let mut attached_targets = get_mcus();
    let mut targets = Vec::new();

    if attached_targets.is_empty() {
        return Err(anyhow!("No targets attached to service (0 MCUs detected)"));
    }

    for (probe_serial, probe_info) in target_settings {
        if let Some((probe_serial, cpu_type)) = attached_targets.remove_entry(&probe_serial) {
            targets.push(Target {
                cpu_type,
                probe_serial,
                probe_alias: probe_info.probe_alias.clone(),
                target_name: probe_info.target_name.clone(),
            });
        } else {
            warn!("Probe with serial '{}' is not attached.", probe_serial.0);
        }
    }

    for (ps, _) in attached_targets {
        warn!(
            "Probe with serial '{}' does not have a configuration.",
            ps.0
        );
    }

    Ok(Targets::new(targets))
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SavedSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_tokens: Option<HashMap<AuthName, AuthToken>>,
    #[serde(default)]
    probe_configs: HashMap<ProbeSerial, ProbeInfo>,
    #[serde(default)]
    server_configs: ServerConfigs,
}

impl std::fmt::Display for SavedSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "")?;
        if let Some(tokens) = &self.auth_tokens {
            writeln!(f, "  - Auth tokens:")?;
            for (name, token) in tokens {
                writeln!(f, "    - {}: {}", name, token)?;
            }
        } else {
            writeln!(f, "  - Auth tokens: None")?;
        }

        writeln!(f, "")?;
        writeln!(f, "  - Probe configs:")?;
        for (serial, conf) in &self.probe_configs {
            writeln!(
                f,
                "    - {}: {{ target_name: {}, probe_alias: {}{} }}",
                serial,
                conf.target_name,
                conf.probe_alias,
                if let Some(speed) = conf.probe_speed_khz {
                    format!(", probe_speed_khz: {}", speed)
                } else {
                    format!("")
                }
            )?;
        }

        writeln!(f, "")?;
        writeln!(f, "  - Server configs:")?;
        writeln!(
            f,
            "    - max_target_timeout: {} seconds",
            self.server_configs.max_target_timeout.0
        )?;
        writeln!(
            f,
            "    - max_jobs_in_queue: {}",
            self.server_configs.max_jobs_in_queue
        )?;

        Ok(())
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfigs {
    #[serde(default)]
    pub max_target_timeout: Timeout,
    #[serde(default = "default_max_jobs_in_queue")]
    pub max_jobs_in_queue: usize,
}

fn default_max_jobs_in_queue() -> usize {
    40
}

/// Timeout in seconds.
#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct Timeout(pub u32);

impl Default for Timeout {
    fn default() -> Self {
        Timeout(30)
    }
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

            if v.target_name.0.is_empty() {
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

    if !args.probe_config.exists() {
        fs::write(
            &args.probe_config,
            &serde_json::to_string_pretty(&SavedSettings::default())?,
        )?;
    }

    let s = fs::read_to_string(&args.probe_config)?;
    let mut settings: SavedSettings = serde_json::from_str(&s)?;

    settings.validate()?;

    println!("Starting server with settings: {}", settings);

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
        server_configs: settings.server_configs,
    })
}
