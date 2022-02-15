use clap::Parser;
use embedded_ci_server::{CpuId, RunOn, TargetName};
use reqwest::Url;
use std::{path::PathBuf, time::Duration};

/// A thin CLI on top of the `embedded-ci-server` API calls
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
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
    #[clap(long)]
    target: Option<String>,

    /// The core on which to run.
    #[clap(long)]
    core: Option<CpuId>,

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

    let run_on = match (args.target, args.core) {
        (Some(target), None) => RunOn::Target(TargetName(target)),
        (None, Some(core)) => RunOn::Core(core),
        (None, None) => {
            println!("Error: Only one of --target or --core can be used at the same time");
            std::process::exit(1);
        }
        (Some(_), Some(_)) => {
            println!("Error: One of --target or --core is required");
            std::process::exit(1);
        }
    };

    Cli {
        server: args.server,
        auth_token: args.auth_token,
        run_on,
        elf_file: args.elf_file,
        timeout: Duration::from_secs(args.timeout as _),
    }
}
