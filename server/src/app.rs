use crate::{
    cli::{ProbeInfo, ServerConfigs},
    runner::{self, RunnerError},
};
use anyhow::anyhow;
use embedded_ci_server::{
    CpuId, JobStatus, ProbeAlias, ProbeSerial, RunJob, RunOn, Target, TargetName, Targets,
};
use log::*;
use rand::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// This is the communication channel between the REST API and the embedded runners.
///
/// Jobs are added to this object by the REST API and are removed by the runners.
#[derive(Debug)]
pub struct RunQueue {
    targets: Targets,
    jobs: Vec<(u32, (JobStatus, RunJob))>,
    max_jobs_in_queue: usize,
}

impl RunQueue {
    /// Create a new run queue based on available targets.
    pub fn new(targets: Targets, max_jobs_in_queue: usize) -> Self {
        RunQueue {
            targets,
            jobs: Vec::new(),
            max_jobs_in_queue,
        }
    }

    /// Find a job with a specific job ID.
    pub fn get_job(&self, id: u32) -> Option<&(u32, (JobStatus, RunJob))> {
        self.jobs.iter().find(|(job_id, _)| *job_id == id)
    }

    /// Find a job with a specific job ID, mutable version.
    pub fn get_job_mut(&mut self, id: u32) -> Option<&mut (u32, (JobStatus, RunJob))> {
        self.jobs.iter_mut().find(|(job_id, _)| *job_id == id)
    }

    /// Get the status of a job ID.
    pub fn get_status(&self, id: u32) -> Option<JobStatus> {
        self.get_job(id).map(|val| val.1 .0.clone())
    }

    /// Get the available targets.
    pub fn get_targets(&self) -> &Targets {
        &self.targets
    }

    pub fn jobs_in_queue(&self) -> usize {
        self.jobs
            .iter()
            .map(|(_, (status, _))| match status {
                JobStatus::WaitingInQueue => 1,
                JobStatus::Running => 1,
                _ => 0,
            })
            .sum()
    }

    /// Register a job to the queue.
    pub fn register_job(&mut self, test: RunJob) -> Result<u32, String> {
        let jobs_in_queue = self.jobs_in_queue();
        if jobs_in_queue >= self.max_jobs_in_queue {
            return Err(format!("Run queue full ({} jobs in queue)", jobs_in_queue));
        }

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
                let candidate: u32 = random();

                if self.get_job(candidate).is_none() {
                    break candidate;
                }
            };

            self.jobs.push((id, (JobStatus::WaitingInQueue, test)));

            info!(
                "Registered job with ID {} ({} jobs in line)",
                id,
                self.jobs_in_queue()
            );

            Ok(id)
        } else {
            let s = match &test.run_on {
                RunOn::ProbeSerial(serial) => {
                    format!("Probe with serial '{}' does not exist", serial.0)
                }
                RunOn::ProbeAlias(alias) => {
                    format!("Probe with alias '{}' does not exist", alias.0)
                }
                RunOn::Target(target_name) => {
                    format!("Target with name '{}' does not exist", target_name.0)
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
        server_configs: ServerConfigs,
    ) {
        let queue = run_queue.lock().unwrap();

        for target in queue.get_targets().all_targets() {
            let probe_config = probe_configs.get(&target.probe_serial).unwrap();
            let mut worker = Worker::from_settings(
                target,
                run_queue.clone(),
                probe_config.probe_speed_khz,
                server_configs.clone(),
            );
            let _worker_handle = tokio::spawn(async move { worker.run().await });
            info!("Started worker for probe {}", target.probe_serial.0);
        }
    }
}

/// Async worker for an embedded target.
struct Worker {
    probe_serial: ProbeSerial,
    probe_alias: ProbeAlias,
    probe_speed_khz: Option<u32>,
    target_name: TargetName,
    cpu_type: CpuId,
    jobs: Arc<Mutex<RunQueue>>,
    server_configs: ServerConfigs,
}

impl Worker {
    /// Create a worker from a target.
    fn from_settings(
        target: &Target,
        jobs: Arc<Mutex<RunQueue>>,
        probe_speed_khz: Option<u32>,
        server_configs: ServerConfigs,
    ) -> Self {
        Worker {
            probe_serial: target.probe_serial.clone(),
            probe_alias: target.probe_alias.clone(),
            probe_speed_khz,
            target_name: target.target_name.clone(),
            cpu_type: target.cpu_type,
            jobs,
            server_configs,
        }
    }

    /// Main async runner for a worker.
    async fn run(&mut self) {
        loop {
            trace!("{}: Run loop for probe", self.probe_serial.0);

            let mut id = None;
            {
                // Find a job

                let mut jobs = self.jobs.lock().unwrap();

                let in_queue = jobs.jobs_in_queue();
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
                            info!(
                                "{}: Started job with ID {} ({} jobs in line)",
                                self.probe_serial.0, test_id, in_queue
                            );
                            id = Some((*test_id, test_spec.clone()));
                            *job_status = JobStatus::Running;

                            break;
                        }
                    }
                }
            }

            if let Some((id, test_spec)) = id {
                // Do the actual work (this is a synchronous operation that can take a long time)
                let target_name = self.target_name.clone();
                let probe_serial = self.probe_serial.clone();
                let probe_speed_khz = self.probe_speed_khz;
                let timeout = self.server_configs.max_target_timeout.0;

                let test_res = tokio::task::spawn_blocking(move || {
                    Worker::run_test(
                        &target_name,
                        &probe_serial,
                        probe_speed_khz,
                        timeout,
                        &test_spec,
                    )
                })
                .await
                .unwrap();

                let mut jobs = self.jobs.lock().unwrap();
                if let Some((_, (job_status, test_spec))) = jobs.get_job_mut(id) {
                    info!("{}: Finished job with ID {}", self.probe_serial.0, id);

                    match test_res {
                        Ok(log) => *job_status = JobStatus::Done { log },
                        Err(e) => *job_status = JobStatus::Error(unroll_error(&e)),
                    }

                    // Delete the binary file, it eats up a lot of space.
                    test_spec.binary_b64 = String::new();
                }
            } else {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    /// Run a job on the worker.
    fn run_test<'a>(
        target_name: &'a TargetName,
        probe_serial: &'a ProbeSerial,
        probe_speed_khz: Option<u32>,
        max_target_timeout: u32,
        test_specification: &RunJob,
    ) -> Result<String, RunnerError> {
        let elf_file = base64::decode(&test_specification.binary_b64)
            .map_err(|_| anyhow!("Firmware is not b64"))?;

        let mut runner =
            runner::Runner::new(&elf_file, &target_name, &probe_serial, probe_speed_khz)?;

        let run = runner.run(Duration::from_secs(
            test_specification
                .timeout_secs
                .min(max_target_timeout)
                .into(),
        ));

        debug!("{}: Runner exit status: {:?}", probe_serial.0, run);

        run
    }
}

/// Unrolls errors.
pub fn unroll_error(e: &dyn std::error::Error) -> String {
    let mut s = String::new();
    let mut level = 0;

    s.push_str(&format!("\nError: {}", e));

    let mut source = e.source();

    if source.is_some() {
        s.push_str("\n\nCaused by:");
    }

    while let Some(e) = source {
        s.push_str(&format!("\n    {}: {}", level, e));
        source = e.source();
        level += 1;
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
            tokio::time::sleep(Duration::from_secs(60)).await;

            let jobs = &mut run_queue.lock().unwrap().jobs;

            let mut first_cleanup = true;

            for id in to_cleanup.drain(..) {
                if first_cleanup {
                    debug!("Running cleanup of finished jobs...");
                    first_cleanup = false;
                }
                trace!("    Cleaning up job ID {}...", id);
                let idx = if let Some(index) = jobs.iter().position(|(job_id, _)| *job_id == id) {
                    index
                } else {
                    continue;
                };
                jobs.remove(idx);
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
