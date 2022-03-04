use clap::Parser;
use embedded_ci_client::{ProbeAlias, ProbeSerial};
use embedded_ci_common::{CpuId, RunOn, TargetName};
use reqwest::Url;
use std::{path::PathBuf, time::Duration};

/// A thin CLI on top of the `embedded-ci-server` API calls
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None, group = clap::ArgGroup::new("options").multiple(false))]
struct Args {
    /// Server address.
    #[clap(
        long,
        default_value = "http://localhost:8000",
        env = "EMBEDDED_CI_SERVER"
    )]
    server: Url,

    /// Optional authorization token with the CI server.
    #[clap(long, env = "EMBEDDED_CI_TOKEN")]
    auth_token: Option<String>,

    /// The target on which to run.
    #[clap(long, group = "options")]
    target: Option<String>,

    /// The probe alias on which to run.
    #[clap(long, group = "options")]
    probe_alias: Option<String>,

    /// The probe serial on which to run.
    #[clap(long, group = "options")]
    probe_serial: Option<String>,

    /// The list of cores on which to run.
    #[clap(long, value_delimiter=',', group = "options")]
    cores: Option<Vec<CpuId>>,

    /// Timeout in seconds.
    #[clap(long, default_value = "30")]
    timeout: u32,

    /// The ELF file to run on the CI server.
    elf_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Cli {
    pub server: Url,
    pub auth_token: Option<String>,
    pub run_on: RunOn,
    pub elf_file: PathBuf,
    pub timeout: Duration,
}

pub fn cli() -> Cli {
    let args = Args::parse();

    let run_on = match (args.target, args.cores, args.probe_alias, args.probe_serial) {
        (Some(target), _, _, _) => RunOn::Target(TargetName(target)),
        (_, Some(cores), _, _) => RunOn::Core(cores),
        (_, _, Some(probe_alias), _) => RunOn::ProbeAlias(ProbeAlias(probe_alias)),
        (_, _, _, Some(probe_serial)) => RunOn::ProbeSerial(ProbeSerial(probe_serial)),
        _ => unreachable!(),
    };

    Cli {
        server: args.server,
        auth_token: args.auth_token,
        run_on,
        elf_file: args.elf_file,
        timeout: Duration::from_secs(args.timeout as _),
    }
}
