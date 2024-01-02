use std::{
    io::{BufReader, Read},
    process::{Command, Stdio},
};

use bzip2::{bufread::BzEncoder, Compression};
use regex::Regex;

#[derive(Debug)]
pub struct LogicAnalyzer {
    bus: u8,
    device: u8,
}

impl LogicAnalyzer {
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
                "Process exited with unsuccessful status code: {:?}",
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
                log::debug!("Found: {la:0x?}");
                Some(la)
            })
            .collect();

        Ok(result)
    }

    pub async fn capture_bzipped(
        &self,
        megasamples: u8,
        megasample_rate: u8,
    ) -> anyhow::Result<Vec<u8>> {
        let mut command = Command::new("sigrok-cli");
        command
            .args([
                "--driver",
                &format!("fx2lafw:conn={}.{}", self.bus, self.device),
            ])
            .args(["--samples", &format!("{}M", megasamples)])
            .args(["--config", &format!("samplerate={}M", megasample_rate)])
            .args(["-O", "binary"])
            .stdout(Stdio::piped());
        log::debug!("Running {command:?}");

        let mut child = command.spawn()?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("No stdout available when spawning sigrok-cli?"))?;
        let child_stdout_handle = tokio::task::spawn_blocking(|| {
            BzEncoder::new(BufReader::new(child_stdout), Compression::default())
                .bytes()
                .collect::<Result<Vec<_>, _>>()
        });
        let child_process_handle = tokio::task::spawn_blocking(move || child.wait());
        let exit_status = child_process_handle.await??;
        if !exit_status.success() {
            return Err(anyhow::anyhow!(
                "Process exited with unsuccessful status code: {:?}",
                exit_status.code()
            ));
        }
        Ok(child_stdout_handle.await??)
    }
}
