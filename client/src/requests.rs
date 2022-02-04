use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CpuId {
    CortexM0,
    CortexM0Plus,
    CortexM1,
    CortexM3,
    CortexM4,
    CortexM7,
    CortexM23,
    CortexM33,
}

impl FromStr for CpuId {
    type Err = &'static str;

    fn from_str(param: &str) -> Result<Self, Self::Err> {
        let v: &str = &param.to_ascii_lowercase();
        Ok(match v {
            "cortexm0" => CpuId::CortexM0,
            "cortexm0plus" => CpuId::CortexM0Plus,
            "cortexm1" => CpuId::CortexM1,
            "cortexm3" => CpuId::CortexM3,
            "cortexm4" => CpuId::CortexM4,
            "cortexm7" => CpuId::CortexM7,
            "cortexm23" => CpuId::CortexM23,
            "cortexm33" => CpuId::CortexM33,
            _ => return Err("Unable to parse"),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RunOn {
    Name(String),
    Target(String),
    Core(CpuId),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunJob {
    pub run_on: RunOn,
    pub binary_b64: String,
    pub timeout_secs: u8,
}

/// The current status of a job.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum JobStatus {
    /// Waiting, an embedded runner has not yet accepted this job.
    WaitingInQueue,
    /// Running, an embedded runner is actively running this job.
    Running,
    /// Done, the job has finished successfully.
    Done { log: String },
    /// Error, the job has finished with error (the string holds the specific error).
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Targets {
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub cpu_type: CpuId,
    pub probe_serial: String,
    pub probe_alias: String,
    pub target_name: String,
}
