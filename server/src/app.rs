use crate::{
    cli::{ProbeInfo, ProbeSerial},
    runner::{self, RunnerError},
    target::{CpuId, Target, Targets},
};
use anyhow::anyhow;
use log::*;
use rand::prelude::*;
use rocket_okapi::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// On which target a job should run on.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub enum RunOn {
    /// Run on a specific probe serial number.
    ProbeSerial(String),
    /// Run on a specific probe alias.
    ProbeAlias(String),
    /// Run on a specific target name.
    Target(String),
    /// Run on a specific core type.
    Core(CpuId),
}

impl RunOn {
    /// Helper to check this parameter.
    fn is_valid(&self) -> bool {
        match self {
            RunOn::ProbeSerial(serial) => !serial.is_empty(),
            RunOn::ProbeAlias(alias) => !alias.is_empty(),
            RunOn::Target(target) => !target.is_empty(),
            RunOn::Core(_) => true,
        }
    }
}

/// A job specification for a run.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct RunJob {
    /// On which embedded target should this job run on.
    pub run_on: RunOn,
    /// The ELF file holding the binary and debug symbols.
    pub binary_b64: String,
    /// Timeout of the job in seconds.
    pub timeout_secs: u8,
}

/// The current status of a job.
#[derive(Debug, JsonSchema, Serialize, Deserialize, Clone, PartialEq, Eq)]
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

/// This is the communication channel between the REST API and the embedded runners.
///
/// Jobs are added to this object by the REST API and are removed by the runners.
#[derive(Debug)]
pub struct RunQueue {
    targets: Targets,
    jobs: HashMap<u32, (JobStatus, RunJob)>,
}

impl RunQueue {
    /// Create a new run queue based on available targets.
    pub fn new(targets: Targets) -> Self {
        RunQueue {
            targets,
            jobs: HashMap::new(),
        }
    }

    /// Get the status of a job ID.
    pub fn get_status(&self, id: u32) -> Option<JobStatus> {
        self.jobs.get(&id).map(|val| val.0.clone())
    }

    /// Get the available targets.
    pub fn get_targets(&self) -> &Targets {
        &self.targets
    }

    /// Register a job to the queue.
    pub fn register_job(&mut self, test: RunJob) -> Result<u32, String> {
        let available = test.run_on.is_valid()
            && match &test.run_on {
                RunOn::ProbeSerial(serial) => self.targets.get_probe(serial).is_some(),
                RunOn::ProbeAlias(alias) => self.targets.get_probe_alias(alias).is_some(),
                RunOn::Target(target_name) => self.targets.get_target(target_name).is_some(),
                RunOn::Core(cpu_id) => self.targets.get_core(cpu_id).is_some(),
            };

        if available {
            let id = loop {
                // Find a free ID
                let candidate = random();

                if self.jobs.get(&candidate).is_none() {
                    break candidate;
                }
            };

            self.jobs.insert(id, (JobStatus::WaitingInQueue, test));

            Ok(id)
        } else {
            let s = match &test.run_on {
                RunOn::ProbeSerial(serial) => {
                    format!("Probe with serial '{}' does not exist", serial)
                }
                RunOn::ProbeAlias(alias) => format!("Probe with alias '{}' does not exist", alias),
                RunOn::Target(target_name) => {
                    format!("Target with name '{}' does not exist", target_name)
                }
                RunOn::Core(cpu_id) => format!("Core of type '{:?}' does not exist", cpu_id),
            };

            Err(s)
        }
    }
}

/// This run the backend.
pub struct Backend {}

impl Backend {
    /// Start the backend job given the run queue (link between REST API and embedded runner) and
    /// probe configs.
    pub async fn run(
        run_queue: Arc<Mutex<RunQueue>>,
        probe_configs: HashMap<ProbeSerial, ProbeInfo>,
    ) {
        let queue = run_queue.lock().unwrap();

        for target in queue.get_targets().all_targets() {
            let probe_config = probe_configs
                .get(&ProbeSerial(target.probe_serial.clone()))
                .unwrap();
            let mut worker =
                Worker::from_target(target, run_queue.clone(), probe_config.probe_speed_khz);
            let _worker_handle = tokio::spawn(async move { worker.run().await });
            info!("Started worker for probe {}", target.probe_serial);
        }
    }
}

/// Async worker for an embedded target.
struct Worker {
    probe_serial: String,
    probe_alias: String,
    probe_speed_khz: Option<u32>,
    target_name: String,
    cpu_type: CpuId,
    jobs: Arc<Mutex<RunQueue>>,
}

impl Worker {
    /// Create a worker from a target.
    fn from_target(
        target: &Target,
        jobs: Arc<Mutex<RunQueue>>,
        probe_speed_khz: Option<u32>,
    ) -> Self {
        Worker {
            probe_serial: target.probe_serial.clone(),
            probe_alias: target.probe_alias.clone(),
            probe_speed_khz,
            target_name: target.target_name.clone(),
            cpu_type: target.cpu_type,
            jobs,
        }
    }

    /// Main async runner for a worker.
    async fn run(&mut self) {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;

            trace!("{}: Run loop for probe", self.probe_serial);

            let mut id = None;
            {
                // Find a job

                let mut jobs = self.jobs.lock().unwrap();

                for (test_id, (job_status, test_spec)) in &mut jobs.jobs {
                    if job_status == &JobStatus::WaitingInQueue {
                        let for_us = test_spec.run_on.is_valid()
                            && match &test_spec.run_on {
                                RunOn::ProbeSerial(serial) => serial == &self.probe_serial,
                                RunOn::ProbeAlias(alias) => alias == &self.probe_alias,
                                RunOn::Target(target_name) => target_name == &self.target_name,
                                RunOn::Core(cpu_type) => cpu_type == &self.cpu_type,
                            };

                        if for_us {
                            info!("{}: Accepted job with ID {}", self.probe_serial, test_id);
                            id = Some((*test_id, test_spec.clone()));
                            *job_status = JobStatus::Running;

                            break;
                        }
                    }
                }
            }

            if let Some((id, test_spec)) = id {
                // Do the actual work
                let test_res = self.run_test(&test_spec);

                let mut jobs = self.jobs.lock().unwrap();
                if let Some((job_status, _test_spec)) = jobs.jobs.get_mut(&id) {
                    info!("{}: Finished job with ID {}", self.probe_serial, id);

                    match test_res {
                        Ok(log) => *job_status = JobStatus::Done { log },
                        Err(e) => *job_status = JobStatus::Error(unroll_error(&e)),
                    }
                }
            }
        }
    }

    /// Run a job on the worker.
    fn run_test(&mut self, test_specification: &RunJob) -> Result<String, RunnerError> {
        let elf_file = base64::decode(&test_specification.binary_b64)
            .map_err(|_| anyhow!("Firmware is not b64"))?;

        let mut runner = runner::Runner::new(
            &elf_file,
            &self.target_name,
            &self.probe_serial,
            self.probe_speed_khz,
        )?;

        let run = runner.run(Duration::from_secs(test_specification.timeout_secs.into()));

        debug!("{}: Runner exit status: {:?}", self.probe_serial, run);

        run
    }
}

fn unroll_error(e: &dyn std::error::Error) -> String {
    let mut s = String::new();
    let mut level = 0;

    s.push_str(&format!("\n{}: {}", level, e));

    let mut source = e.source();

    while let Some(e) = source {
        level += 1;
        s.push_str(&format!("\n{}: {}", level, e));
        source = e.source();
    }

    s
}

/// This handles the cleanup of finished jobs.
pub struct Cleanup {}

impl Cleanup {
    /// Start the cleanup job given the run queue, this cleans up old and expired jobs over time.
    pub async fn run(run_queue: Arc<Mutex<RunQueue>>) {
        info!("Starting job cleanup worker");

        let mut to_cleanup = Vec::new();
        loop {
            // We do cleanup once per minute
            tokio::time::sleep(Duration::from_secs(60)).await;

            let jobs = &mut run_queue.lock().unwrap().jobs;

            debug!("Running cleanup of finished jobs...");

            for id in to_cleanup.drain(..) {
                debug!("    Cleaning up job ID {}...", id);
                jobs.remove_entry(&id);
            }

            jobs.shrink_to_fit();

            for (job_id, (job_status, _)) in jobs {
                match job_status {
                    JobStatus::Done { log: _ } | JobStatus::Error(_) => {
                        // Add to next round of cleanup
                        to_cleanup.push(*job_id);
                    }
                    _ => {}
                }
            }
        }
    }
}
