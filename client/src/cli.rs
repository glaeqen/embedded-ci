use crate::requests::{CpuId, RunOn};
use clap::Parser;
use reqwest::Url;
use std::path::PathBuf;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, default_value = "http://localhost:8000")]
    server: Url,

    #[clap(short, long)]
    auth_token: Option<String>,

    #[clap(short, long)]
    target: Option<String>,

    #[clap(short, long)]
    core: Option<CpuId>,

    elf_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Cli {
    pub server: Url,
    pub auth_token: Option<String>,
    pub run_on: RunOn,
    pub elf_file: PathBuf,
}

pub fn cli() -> Cli {
    let args = Args::parse();

    let run_on = match (args.target, args.core) {
        (Some(target), None) => RunOn::Target(target),
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
    }
}
