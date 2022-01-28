use crate::{
    runner,
    target::{CpuId, Target, Targets},
};
use anyhow::anyhow;
use log::*;
use probe_rs::flashing::{download_file_with_options, DownloadOptions, Format};
use probe_rs::{CoreStatus, DebugProbeError, HaltReason, Probe, ProbeCreationError};
use probe_rs_rtt::Rtt;
use rand::prelude::*;
use rocket_okapi::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{fs, thread};

#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub enum RunOn {
    Probe(String),
    Target(String),
    Core(CpuId),
}

#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct RunTest {
    pub run_on: RunOn,
    pub binary_b64: String,
    pub timeout_secs: u8,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum JobStatus {
    WaitingInQueue,
    Running,
    Done { log: String },
    Error(String),
}

#[derive(Debug)]
pub struct RunQueue {
    targets: Targets,
    jobs: HashMap<u32, (JobStatus, RunTest)>,
}

impl RunQueue {
    pub fn new(targets: Targets) -> Self {
        RunQueue {
            targets,
            jobs: HashMap::new(),
        }
    }

    pub fn get_status(&self, id: u32) -> Option<JobStatus> {
        self.jobs.get(&id).map(|val| val.0.clone())
    }

    pub fn get_targets(&self) -> &Targets {
        &self.targets
    }

    pub fn register_job(&mut self, test: RunTest) -> Result<u32, String> {
        let available = match &test.run_on {
            RunOn::Probe(serial) => self.targets.get_probe(serial).is_some(),
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
                RunOn::Probe(serial) => format!("Probe with serial '{}' does not exist", serial),
                RunOn::Target(target_name) => {
                    format!("Target with name '{}' does not exist", target_name)
                }
                RunOn::Core(cpu_id) => format!("Core of type '{:?}' does not exist", cpu_id),
            };

            Err(s)
        }
    }
}

pub struct Backend {}

impl Backend {
    pub async fn run(run_queue: Arc<Mutex<RunQueue>>) {
        let queue = run_queue.lock().unwrap();

        for (worker_no, target) in queue.get_targets().all_targets().iter().enumerate() {
            let mut worker = Worker::from_target(worker_no, target, run_queue.clone());
            let worker_handle = tokio::spawn(async move { worker.run().await });
            info!("Started worker for probe {}", target.probe_serial);
        }
    }
}

struct Worker {
    probe_serial: String,
    target_name: String,
    cpu_type: CpuId,
    jobs: Arc<Mutex<RunQueue>>,
}

impl Worker {
    fn from_target(worker_no: usize, target: &Target, jobs: Arc<Mutex<RunQueue>>) -> Self {
        Worker {
            probe_serial: target.probe_serial.clone(),
            target_name: target.target_name.clone(),
            cpu_type: target.cpu_type,
            jobs,
        }
    }

    async fn run(&mut self) {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;

            trace!("{}: Run loop for probe", self.probe_serial);

            let mut id = None;
            {
                // Find a job

                let mut jobs = self.jobs.lock().unwrap();

                for (test_id, (job_status, test_spec)) in &mut jobs.jobs {
                    if job_status == &JobStatus::WaitingInQueue {
                        let for_us = match &test_spec.run_on {
                            RunOn::Probe(serial) => serial == &self.probe_serial,
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
                        Err(e) => *job_status = JobStatus::Error(e.to_string()),
                    }
                }
            }
        }
    }

    fn run_test(&mut self, test_specification: &RunTest) -> Result<String, probe_rs::Error> {
        let elf_file = base64::decode(&test_specification.binary_b64)
            .map_err(|_| anyhow!("Firmware is not b64"))?;

        let mut runner = runner::Runner::new(&elf_file, &self.target_name, &self.probe_serial)?;

        Ok(runner.run(Duration::from_secs(test_specification.timeout_secs.into()))?)
    }
}
