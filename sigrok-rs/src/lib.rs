use std::{
    io::{BufReader, Read},
    process::{Command, Stdio},
    time::Duration,
};

use bzip2::{bufread::BzEncoder, Compression};
use regex::Regex;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct LogicAnalyzer {
    bus: u8,
    device: u8,
}

impl LogicAnalyzer {
    // TODO: This probably should be a singleton type of operation (maybe)
    pub fn all() -> anyhow::Result<Vec<Self>> {
        let mut command = Command::new("sigrok-cli");
        // Should be fast enough not to warrant the `spawn_blocking`
        let output = command
            .args(["--driver", "fx2lafw"])
            .args(["--scan"])
            .stderr(Stdio::inherit())
            .output()?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Process exited with unsuccessful exit code: {:?}",
                output.status.code()
            ));
        }
        let regex_str = "fx2lafw:conn=([0-9]+).([0-9]+)";
        log::trace!("Applying regex: `{regex_str}` on `sigrok-cli --scan` output");
        let regex = Regex::new(regex_str)?;
        let result = std::str::from_utf8(&output.stdout)?
            .lines()
            .filter_map(|line| {
                log::trace!("Trying with: {line}");
                let capture = regex.captures(line)?;
                log::trace!("Success: {capture:?}");
                let bus = capture.get(1)?.as_str().parse().ok()?;
                let device = capture.get(2)?.as_str().parse().ok()?;
                let la = Self { bus, device };
                log::debug!("Found: {la:?}");
                Some(la)
            })
            .collect();

        Ok(result)
    }

    /// Initiates the capture of data from the logic analyzer
    ///
    /// To get the data from the logic analyzer, call [`ActiveCapture::stop_capture`].
    pub async fn start_capture<'la>(
        &'la mut self,
        samplerate_mhz: u8,
    ) -> anyhow::Result<ActiveCapture<'la>> {
        let mut command = Command::new("sigrok-cli");
        command
            .args([
                "--driver",
                &format!("fx2lafw:conn={}.{}", self.bus, self.device),
            ])
            .arg("--continuous")
            .args(["--config", &format!("samplerate={}M", samplerate_mhz)])
            .args(["-O", "binary"])
            .stdout(Stdio::piped());

        log::debug!("Starting {command:?}");

        let mut process_handle = KillOnDropProcessHandle {
            inner: command.spawn()?,
        };
        let child_stdout = process_handle
            .inner
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("No stdout available when spawning sigrok-cli?"))?;
        let process_stdout_handle = tokio::task::spawn_blocking(|| {
            BzEncoder::new(BufReader::new(child_stdout), Compression::default())
                .bytes()
                .collect()
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        error_if_process_exited_prematurely(&mut process_handle.inner)?;

        Ok(ActiveCapture {
            __: core::marker::PhantomData,
            process_handle,
            process_stdout_handle,
        })
    }
}

fn error_if_process_exited_prematurely(process: &mut std::process::Child) -> anyhow::Result<()> {
    match process.try_wait()? {
        Some(exit_status) => Err(anyhow::anyhow!(
            "Process exited prematurely with {exit_status:?}"
        )),
        None => Ok(()),
    }
}

/// Wrapper for the `Child` process handle that kills the process on drop
///
/// By default, [`std::process::Child`] does not kill the underlying process and
/// thus leaves it running in the background.
///
/// This wrapper is not bulletproof in case when the drop handler is not called
struct KillOnDropProcessHandle {
    inner: std::process::Child,
}

impl Drop for KillOnDropProcessHandle {
    fn drop(&mut self) {
        // More harsh then SIGTERM (uses SIGKILL underneath) but probably should be fine.
        // API makes it legal to kill already killed process, it's a noop.
        self.inner.kill().ok();
        // Waiting on the killed process is necessary in order to avoid creation of a
        // zombie process
        self.inner.wait().ok();
    }
}

type ProcessStdoutJoinHandle = JoinHandle<std::io::Result<Vec<u8>>>;

/// Struct representing an on-going data capture
///
/// This struct is created by calling [`LogicAnalyzer::start_capture`]
pub struct ActiveCapture<'la> {
    __: core::marker::PhantomData<&'la ()>,
    process_handle: KillOnDropProcessHandle,
    process_stdout_handle: ProcessStdoutJoinHandle,
}

impl<'la> ActiveCapture<'la> {
    /// Stop the ongoing data capture.
    ///
    /// Data is captured in a bz2-encoded raw binary logic data format of sigrok
    pub async fn stop_capture(mut self) -> anyhow::Result<Vec<u8>> {
        error_if_process_exited_prematurely(&mut self.process_handle.inner)?;
        core::mem::drop(self.process_handle);
        Ok(self.process_stdout_handle.await??)
    }
}
