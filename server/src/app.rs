use crate::{
    cli::{ProbeInfo, ServerConfigs},
    runner,
};
use embedded_ci_common::{
    job::{self, RunResultDetails},
    ProbeSerial, ServerStatus,
};
use log::*;
use sigrok_rs::LogicAnalyzer;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
};
use tokio::sync::mpsc;

/// Start the backend job given the run queue (link between REST API and embedded runner) and
/// probe configs.
pub async fn run(
    mut register_job_rx: mpsc::Receiver<job::Job>,
    finished_job_tx: mpsc::Sender<job::JobResult>,
    server_status: Arc<Mutex<ServerStatus>>,
    probe_configs: HashMap<ProbeSerial, ProbeInfo>,
    server_configs: ServerConfigs,
) {
    let max_target_timeout = server_configs.max_target_timeout;
    loop {
        let job = register_job_rx.recv().await.unwrap();
        let job_id = job.id;
        info!("{job_id}: received");
        server_status.lock().unwrap().job_started(job_id);
        let sync_barrier = crossbeam::sync::WaitGroup::new();
        let mut job_result = job::JobResult::empty_from_job(&job);
        let mut runs = Vec::new();
        let timeout = Duration::from_secs(job.timeout.as_secs().min(max_target_timeout.0 as _));
        let probe_mutex = Arc::new(Mutex::new(()));
        for task in job.tasks.into_iter() {
            for target in task.targets.into_iter() {
                let probe_speed_khz = probe_configs
                    .get(&target.probe_serial)
                    .and_then(|pc| pc.probe_speed_khz);
                let task_id = task.id;
                let run_id = target.probe_serial.clone();
                debug!("{job_id}/{task_id}/{run_id}: setting up");
                runs.push((
                    task_id,
                    run_id.clone(),
                    tokio::task::spawn_blocking({
                        let task_binary = task.binary.clone();
                        let sync_barrier = sync_barrier.clone();
                        let probe_mutex = probe_mutex.clone();
                        move || {
                            debug!("{job_id}/{task_id}/{run_id}: started");
                            let mut runner = runner::Runner::new(
                                &task_binary,
                                &target.target_name,
                                &target.probe_serial,
                                probe_speed_khz,
                            )?;
                            runner.run(&probe_mutex, sync_barrier, timeout)
                        }
                    }),
                ));
            }
        }
        let mut logic_analyzers = LogicAnalyzer::all().unwrap_or_default();
        let mut active_captures = Vec::new();
        for logic_analyzer in logic_analyzers.iter_mut() {
            match logic_analyzer
                .start_capture(24 /* TODO: Do not hardcode? */)
                .await
            {
                Ok(active_capture) => active_captures.push(active_capture),
                Err(e) => error!("Failed to start the logic analyzer data capture: {e}"),
            }
        }
        if let Err(e) = tokio::task::spawn_blocking(move || sync_barrier.wait()).await {
            error!("Failed to join the blocking thread: {e}");
        }
        for (task_id, run_id, run) in runs.into_iter() {
            let run_outcome_from_runner = run.await.unwrap();
            info!("{job_id}/{task_id}/{run_id}: finished");
            debug!(
                "{job_id}/{task_id}/{run_id}: result: {:?}",
                &run_outcome_from_runner
            );
            let run_result = job_result
                .task_mut_by_id(task_id)
                .unwrap()
                .run_mut_by_probe_serial(&run_id)
                .unwrap();
            run_result.result = match run_outcome_from_runner {
                Ok(logs) => RunResultDetails::Success { logs },
                Err(error) => RunResultDetails::Failure {
                    error: error.to_string(),
                },
            };
        }
        for active_capture in active_captures.into_iter() {
            match active_capture.stop_capture().await {
                Ok(data) => {
                    job_result.logic_analyzer_capture.push(base64::encode(data));
                }
                Err(e) => error!("Failed to stop the logic analyzer capture: {e}"),
            }
        }
        // Should be ok to await here as a concurrent job is expected to pick the messages up quickly
        match finished_job_tx.send(job_result).await {
            Ok(_) => server_status.lock().unwrap().job_finished(job_id),
            Err(error) => error!("Sending of the finished job failed: {:?}", error),
        }
    }
}

// TODO: To be removed?
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
/// A default queue subscriber that receives finished jobs and populates them
/// in the FIFO for the REST API to leniently consume
pub async fn finished_job_collector(
    finished_job_queue: Arc<Mutex<VecDeque<job::JobResult>>>,
    mut finished_job_rx: mpsc::Receiver<job::JobResult>,
    server_status: Arc<Mutex<ServerStatus>>,
    max_jobs_in_queue: usize,
) {
    loop {
        // Should never lag behind + if sender is closed program should terminate; thus unwrap()
        let finished_job = finished_job_rx.recv().await.unwrap();
        debug!(
            "Moving the job result of id: {} into the finished queue",
            finished_job.id
        );
        // Should never fail
        let mut finished_job_queue = finished_job_queue.lock().unwrap();
        match finished_job_queue.len().cmp(&max_jobs_in_queue) {
            Ordering::Less => finished_job_queue.push_back(finished_job),
            Ordering::Equal => {
                // Cannot fail, holding a mutex between the len check and pop_front
                let dropped_job = finished_job_queue.pop_front().unwrap();
                server_status.lock().unwrap().job_cleared(dropped_job.id);
                trace!(
                    "Queue full, dropping finished job with id: {}",
                    dropped_job.id
                );
                finished_job_queue.push_back(finished_job)
            }
            Ordering::Greater => unreachable!("Queue length longer than max allowed"),
        }
    }
}
