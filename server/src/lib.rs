#![warn(missing_docs)]

//! Here common parts of the embedded-ci is available that are useful for both client and server.
//! Most are related to messages send over the REST API.

use anyhow::anyhow;
use num_enum::TryFromPrimitive;
use rocket::request::FromParam;
use rocket_okapi::JsonSchema;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// On which target a job should run on.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub enum RunOn {
    /// Run on a specific probe serial number.
    ProbeSerial(ProbeSerial),
    /// Run on a specific probe alias.
    ProbeAlias(ProbeAlias),
    /// Run on a specific target name.
    Target(TargetName),
    /// Run on a specific core type.
    Core(CpuId),
}

impl RunOn {
    /// Helper to check this parameter.
    pub fn is_valid(&self) -> bool {
        match self {
            RunOn::ProbeSerial(serial) => !serial.0.is_empty(),
            RunOn::ProbeAlias(alias) => !alias.0.is_empty(),
            RunOn::Target(target) => !target.0.is_empty(),
            RunOn::Core(_) => true,
        }
    }
}

/// A job specification for a run.
#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize)]
pub struct RunJob {
    /// On which embedded target should this job run on.
    pub run_on: RunOn,
    /// The ELF file holding the binary and debug symbols.
    pub binary_b64: String,
    /// Timeout of the job in seconds.
    pub timeout_secs: u8,
}

/// The current status of a job.
#[derive(JsonSchema, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum JobStatus {
    /// Waiting, an embedded runner has not yet accepted this job.
    WaitingInQueue,
    /// Running, an embedded runner is actively running this job.
    Running,
    /// Done, the job has finished successfully and a log is available.
    Done {
        #[allow(missing_docs)]
        log: String,
    },
    /// Error, the job has finished with error (the string holds the specific error).
    Error(String),
}

/// The available types of CPUs this service supports.
#[derive(
    Debug, Clone, Copy, TryFromPrimitive, Hash, PartialEq, Eq, JsonSchema, Serialize, Deserialize,
)]
#[allow(missing_docs)]
#[repr(u32)]
pub enum CpuId {
    CortexM0 = 0xc20,
    CortexM0Plus = 0xc60,
    CortexM1 = 0xc21,
    CortexM3 = 0xc23,
    CortexM4 = 0xc24,
    CortexM7 = 0xc27,
    CortexM23 = 0xd20,
    CortexM33 = 0xd21,
}

impl FromParam<'_> for CpuId {
    type Error = anyhow::Error;

    fn from_param(param: &str) -> Result<Self, Self::Error> {
        Self::from_str(param)
    }
}

impl FromStr for CpuId {
    type Err = anyhow::Error;

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
            _ => return Err(anyhow!("Unable to parse '{}' to CpuId", v)),
        })
    }
}

/// The definition of an embedded target.
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    /// CPU type of the target.
    pub cpu_type: CpuId,
    /// The probe serial on which the target is connected.
    pub probe_serial: ProbeSerial,
    /// An alias for the probe.
    pub probe_alias: ProbeAlias,
    /// The chip name of the target.
    pub target_name: TargetName,
}

/// A list of targets.
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Targets {
    targets: Vec<Target>,
}

impl Targets {
    /// New from a list of targets.
    pub fn new(targets: Vec<Target>) -> Self {
        Targets { targets }
    }

    /// Find the first core of a specific type in the target list.
    pub fn get_core(&self, core: &CpuId) -> Option<&Target> {
        self.targets.iter().find(|target| &target.cpu_type == core)
    }

    /// Find the first target with a specific probe serial in the target list.
    pub fn get_probe(&self, probe_serial: &ProbeSerial) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.probe_serial == probe_serial)
    }

    /// Find the first target with a specific name in the target list.
    pub fn get_target(&self, target_name: &TargetName) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.target_name == target_name)
    }

    /// Find the first target with a specific probe alias in the target list.
    pub fn get_probe_alias(&self, probe_alias: &ProbeAlias) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.probe_alias == probe_alias)
    }

    /// Get all targets.
    pub fn all_targets(&self) -> &[Target] {
        &self.targets
    }
}

/// Probe serial wrapper.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash, Default,
)]

pub struct ProbeSerial(pub String);

/// Probe alias wrapper.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash, Default,
)]
pub struct ProbeAlias(pub String);

/// Target name wrapper.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash, Default,
)]
pub struct TargetName(pub String);

/// Name of an authorization token wrapper.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct AuthName(pub String);

/// Authorization token wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthToken(pub String);
