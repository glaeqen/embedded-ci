#![warn(missing_docs)]

//! Here common parts of the embedded-ci is available that are useful for both client and server.
//! Most are related to messages send over the REST API.

pub use uuid::Uuid;

pub mod job;

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    hash::Hash,
};

/// Current status of the server
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ServerStatus {
    current_job: Option<Uuid>,
    jobs_in_queue: VecDeque<Uuid>,
    jobs_finished: HashSet<Uuid>,
}

/// Status of a queried job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    /// Job has not been found in a system
    NotFound,
    /// Job is currently in the queue awaiting execution
    InQueue,
    /// Job is currently being run
    Running,
    /// Job has finished and its result is available
    Finished,
}

macro_rules! error_if_not_eq {
    ($l:expr, $r:expr) => {{
        if $l != $r {
            log::error!(
                "assertions failed: {} != {}",
                stringify!($l),
                stringify!($r)
            )
        }
    }};
}
macro_rules! error_if_eq {
    ($l:expr, $r:expr) => {{
        if $l == $r {
            log::error!(
                "assertions failed: {} == {}",
                stringify!($l),
                stringify!($r)
            )
        }
    }};
}
macro_rules! error_if_not {
    ($v:expr) => {{
        if !$v {
            log::error!("assertions failed: !{}", stringify!($v))
        }
    }};
}

impl ServerStatus {
    /// Marks the enqueued job as started
    ///
    /// Assumes the oldest `id` in the queue to be `id`
    pub fn job_started(&mut self, id: Uuid) {
        let oldest_job = self.jobs_in_queue.pop_front().unwrap();
        error_if_not_eq!(oldest_job, id);
        error_if_not_eq!(self.current_job.replace(id), None);
    }
    /// Marks the running job as finished
    ///
    /// Assumes `job_started` called with the same `id`
    pub fn job_finished(&mut self, id: Uuid) {
        error_if_not_eq!(self.current_job.take(), Some(id));
        error_if_not!(self.jobs_finished.insert(id));
    }
    /// Removes a finished job
    ///
    /// Assumes `job_finished` called with the same `id`
    pub fn job_cleared(&mut self, id: Uuid) {
        error_if_not!(self.jobs_finished.remove(&id))
    }
    /// Creates a new job entry and marks it as enqueued
    ///
    /// Assumes `id` is not currently enqueued, running nor finished (must be cleared)
    pub fn job_enqueued(&mut self, id: Uuid) {
        error_if_eq!(self.current_job, Some(id));
        error_if_not!(!self.jobs_in_queue.contains(&id));
        error_if_not!(!self.jobs_finished.contains(&id));
        self.jobs_in_queue.push_back(id);
    }
    /// Job status getter
    pub fn job_status(&self, id: Uuid) -> JobStatus {
        if self.current_job == Some(id) {
            return JobStatus::Running;
        }
        if self.jobs_in_queue.contains(&id) {
            return JobStatus::InQueue;
        }
        if self.jobs_finished.contains(&id) {
            return JobStatus::Finished;
        }
        return JobStatus::NotFound;
    }
}

/// On which target a job should run on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOn {
    /// Run on probes with given serial numbers.
    ProbeSerials(Vec<ProbeSerial>),
    /// Run on probes with given aliases.
    ProbeAliases(Vec<ProbeAlias>),
    /// Run on a specific target name.
    Targets(Vec<TargetName>),
    /// Run on a specific list of core types.
    Groups(Vec<TargetGroup>),
}

/// The definition of an embedded target.
///
/// `probe_serial` must be unique for all targets
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    /// The probe serial on which the target is connected.
    pub probe_serial: ProbeSerial,
    /// An alias for the probe.
    pub probe_alias: ProbeAlias,
    /// The chip name of the target.
    pub target_name: TargetName,
    /// Groups which given target belongs to
    pub groups: UnordEqVec<TargetGroup>,
}

/// Vector wrapper which has a custom PartialEq implementation which ignores
/// element ordering.
#[derive(Debug, Clone, Serialize, Deserialize, Eq)]
pub struct UnordEqVec<T: PartialEq>(Vec<T>);

impl<T: PartialEq> Default for UnordEqVec<T> {
    fn default() -> Self {
        Vec::default().into()
    }
}

impl<T: PartialEq> PartialEq for UnordEqVec<T> {
    fn eq(&self, other: &Self) -> bool {
        // Checking both ways to satisfy `Eq`
        self.iter().all(|v| other.contains(v)) && other.iter().all(|v| self.contains(v))
    }
}

impl<T: PartialEq> From<Vec<T>> for UnordEqVec<T> {
    fn from(value: Vec<T>) -> Self {
        Self(value)
    }
}

impl<T: PartialEq> From<UnordEqVec<T>> for Vec<T> {
    fn from(value: UnordEqVec<T>) -> Self {
        value.0
    }
}

impl<T: PartialEq> std::ops::Deref for UnordEqVec<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: PartialEq> std::ops::DerefMut for UnordEqVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A list of targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Targets {
    targets: Vec<Target>,
}

impl From<Vec<Target>> for Targets {
    fn from(targets: Vec<Target>) -> Self {
        Self { targets }
    }
}

impl Targets {
    /// Create a new instance of [`Targets`]
    pub fn new() -> Self {
        Vec::new().into()
    }

    /// Push while making sure that a new target has a unique probe serial
    pub fn push(&mut self, target: Target) -> anyhow::Result<()> {
        if self
            .targets
            .iter()
            .any(|t| t.probe_serial == target.probe_serial)
        {
            Err(anyhow::anyhow!(
                "probe serial '{}' already pushed",
                target.probe_serial
            ))?;
        }
        self.targets.push(target);
        Ok(())
    }

    /// Find the target with a specific probe serial in the target list.
    pub fn find_by_probe_serial(&self, probe_serial: &ProbeSerial) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.probe_serial == probe_serial)
    }

    /// Find the target with a specific name in the target list.
    pub fn find_by_target_name(&self, target_name: &TargetName) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.target_name == target_name)
    }

    /// Find the target with a specific probe alias in the target list.
    pub fn find_by_probe_alias(&self, probe_alias: &ProbeAlias) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.probe_alias == probe_alias)
    }

    /// Return an iterator over targets that belong to the specified group.
    pub fn find_by_group<'a>(
        &'a self,
        group: &'a TargetGroup,
    ) -> impl Iterator<Item = &Target> + 'a {
        self.targets
            .iter()
            .filter(|target| target.groups.contains(group))
    }

    /// Get all targets.
    pub fn all_targets(&self) -> &[Target] {
        &self.targets[..]
    }
}

/// Probe serial wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ProbeSerial(pub String);

impl std::fmt::Display for ProbeSerial {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Probe alias wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ProbeAlias(pub String);

impl std::fmt::Display for ProbeAlias {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Target name wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TargetName(pub String);

impl std::fmt::Display for TargetName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Group that the target belongs to.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TargetGroup(pub String);

impl std::fmt::Display for TargetGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Name of an authorization token wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthName(pub String);

impl std::fmt::Display for AuthName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Authorization token wrapper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthToken(pub String);

impl std::fmt::Display for AuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
