use anyhow::anyhow;
use defmt_decoder::{DecodeError, Locations as DefmtLocations, Table as DefmtTable};
use embedded_ci_common::{ProbeSerial, TargetName};
use log::{debug, error, warn};
use object::{File, Object, ObjectSection, ObjectSymbol};
use probe_rs::rtt::{Error as RttError, Rtt, ScanRegion, UpChannel};
use probe_rs::{
    flashing::{DownloadOptions, FileDownloadError, FlashError},
    MemoryInterface, RegisterId, Session,
};
use probe_rs::{CoreStatus, DebugProbeError, HaltReason, Probe, ProbeCreationError};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use std::{io::Cursor, sync::Arc};

use crate::app::unroll_error;

const THUMB_BIT: u32 = 1;
const SP: RegisterId = RegisterId(13);
const LR: RegisterId = RegisterId(14);
const PC: RegisterId = RegisterId(15);
const PSR: RegisterId = RegisterId(16);
const VTOR: Address = Address(0xE000ED08);

/// Error definitions for runner.
#[derive(thiserror::Error, Debug)]
pub enum RunnerError {
    #[error("ELF file error: {0}")]
    ElfError(String),
    #[error("A flashing error occurred")]
    FlashError(#[from] FlashError),
    #[error("A file download error occurred")]
    FileDownloadError(#[from] FileDownloadError),
    #[error("A debug probe error occurred")]
    DebugProbeError(#[from] DebugProbeError),
    #[error("The app started, but did not reach to main")]
    UnableToReachMain(#[source] probe_rs::Error),
    #[error("A runner error occurred")]
    ProbeRs(#[from] probe_rs::Error),
    #[error("An RTT error occurred")]
    ProbeRsRtt(#[from] RttError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// Internal helper to keep addresses and raw `u32`s apart.
struct Address(pub u32);

/// After the can of the binary is complete this enum holds the best guess of the kind of RTT
/// that is used by the binary.
enum RttType {
    Defmt {
        table: DefmtTable,
        _locations: DefmtLocations,
    },
    PlainText,
}

/// The main runner for embedded targets.
///
/// From here all access and handling of the embedded target happens as it's run by the service.
pub struct Runner<'a> {
    target_name: &'a TargetName,
    probe_serial: &'a ProbeSerial,
    probe_speed_khz: Option<u32>,
    from_ram: bool,
    symbols: Symbols,
    vector_table: VectorTable,
    rtt_type: RttType,
    elf_bytes: &'a [u8],
}

/// Holds important symbol addresses.
struct Symbols {
    main: Address,
    rtt: Address,
}

/// Holds important vector table addresses.
struct VectorTable {
    start: Address,
    stack_pointer: Address,
    reset: Address,
    hardfault: Address,
}

impl<'a> Runner<'a> {
    /// Create a new runner, for running a binary on a target, based on the ELF files and settings.
    pub fn new(
        elf_bytes: &'a [u8],
        target_name: &'a TargetName,
        probe_serial: &'a ProbeSerial,
        probe_speed_khz: Option<u32>,
    ) -> Result<Runner<'a>, RunnerError> {
        let elf = File::parse(elf_bytes)
            .map_err(|e| anyhow!("ELF parsing error, file is not an ELF file: '{}'", e))?;

        let mut rtt = None;
        let mut main = None;

        for symbol in elf.symbols() {
            let name = match symbol.name() {
                Ok(name) => name,
                Err(_) => continue,
            };

            if name == "main" {
                main = Some(symbol.address() as u32 & !THUMB_BIT);
            }

            if name == "_SEGGER_RTT" {
                rtt = Some(symbol.address() as u32);
            }

            if main.is_some() && rtt.is_some() {
                break;
            }
        }

        let symbols = Symbols {
            main: Address(main.ok_or(anyhow!("'main' symbol not found"))?),
            rtt: Address(rtt.ok_or(anyhow!(
                "'_SEGGER_RTT' symbol not found, without RTT this CI tool will not work"
            ))?),
        };

        let important_sections = [".vector_table", ".text", ".rodata", ".data"];
        let mut vector_table = None;
        let mut from_ram = false;

        for section in elf.sections() {
            let name = match section.name() {
                Ok(name) => name,
                Err(_) => continue,
            };

            let addr = section.address();

            if important_sections.contains(&name) {
                if addr % 4 != 0 {
                    // Can sections be unaligned?
                    return Err(RunnerError::ElfError(format!(
                        "Section '{}' is not 4 byte aligned",
                        name
                    )));
                }

                // If it is the vector table, get important addresses from it
                if name == ".vector_table" {
                    let data = section.data().map_err(|_| {
                        RunnerError::ElfError(format!("There is no data in section '{}'", name))
                    })?;
                    if data.len() < 16 {
                        return Err(RunnerError::ElfError(format!(
                            "Section '{}' is too small, size = {} bytes",
                            name,
                            data.len()
                        )));
                    }

                    let vt: Vec<_> = data
                        .chunks_exact(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect();

                    vector_table = Some(VectorTable {
                        start: Address(addr.try_into().map_err(|_| {
                            RunnerError::ElfError(format!(
                                "The address of section '{}' is not 32-bit",
                                name
                            ))
                        })?),
                        stack_pointer: Address(vt[0]),
                        reset: Address(vt[1]),
                        hardfault: Address(vt[3]),
                    });

                    from_ram = addr >= 0x2000_0000;
                }
            }
        }

        let rtt_type = if let Some(table) = defmt_decoder::Table::parse(&elf_bytes)? {
            let locations = table.get_locations(&elf_bytes)?;

            // TODO: This does not seem like it should be a hard error?
            // if !table.is_empty() && locations.is_empty() {
            //     return Err(RunnerError::ElfError(
            //         "'.defmt' symbol found but not enough debug information for defmt, enable debug symbols (debug = 2)".into()
            //     ));
            // } else {
            RttType::Defmt {
                table,
                _locations: locations,
            }
            //}
        } else {
            // The defmt table parsing returned none, so there is no `.defmt` section
            RttType::PlainText
        };

        Ok(Runner {
            target_name,
            probe_serial,
            probe_speed_khz,
            from_ram,
            symbols,
            vector_table: vector_table.ok_or(anyhow!("'.vector_table' section not found"))?,
            rtt_type,
            elf_bytes,
        })
    }

    /// Run the `Runner` to completion with a timeout.
    pub fn run(
        &mut self,
        probe_mutex: &Arc<Mutex<()>>,
        timeout: Duration,
    ) -> Result<String, RunnerError> {
        let probe = self.get_probe(probe_mutex, self.probe_speed_khz)?;

        debug!("{}: Attaching to target", self.probe_serial);
        // First we try to connect normally
        let mut session = match probe.attach(&self.target_name.0, Default::default()) {
            Ok(v) => v,
            Err(e) => {
                // If that fails we fall back to a connect under reset attach
                warn!(
                    "{}: Attach failed ({}), trying with attach under reset...",
                    self.probe_serial, e
                );

                let probe = self.get_probe(probe_mutex, self.probe_speed_khz)?;
                probe
                    .attach_under_reset(&self.target_name.0, Default::default())
                    .map_err(|_| {
                        anyhow!(
                        "Unable to attach to the target, both normal and attach under reset failed"
                    )
                    })?
            }
        };

        debug!("{}: Starting download of ELF", self.probe_serial);
        {
            session.core(0)?.reset_and_halt(Duration::from_secs(3))?;

            let mut opt = DownloadOptions::default();
            opt.verify = true;
            opt.keep_unwritten_bytes = true;

            let mut loader = session.target().flash_loader();
            loader.load_elf_data(&mut Cursor::new(&self.elf_bytes))?;

            loader.commit(&mut session, opt)?;
        }
        debug!("{}: Done!", self.probe_serial);

        let mut core = session.core(0)?;

        if self.from_ram {
            // Fix for ECC RAM, do a dummy write. Thanks to @dirbaio for finding
            let data = core.read_word_32(self.vector_table.start.0 as _)?;
            core.write_word_32(self.vector_table.start.0 as _, data)?;
        }

        core.reset_and_halt(Duration::from_secs(3))?;

        // Check so we have some breakpoint units
        if core.available_breakpoint_units()? == 0 {
            error!(
                "{}: The target does not have any HW breakpoint units?!?! Aborting.",
                self.probe_serial
            );
            return Err(anyhow!(
                "The target does not have any HW breakpoint units?! Aborting."
            ))?;
        }

        debug!("{}: Starting target", self.probe_serial);
        if self.from_ram {
            debug!(
                "{}: Running from RAM (will not halt at main)",
                self.probe_serial
            );
            core.write_core_reg(PC, self.vector_table.reset.0)
                .map_err(|e| RunnerError::UnableToReachMain(e))?;
            core.write_core_reg(SP, self.vector_table.stack_pointer.0)
                .map_err(|e| RunnerError::UnableToReachMain(e))?;
            core.write_word_32(VTOR.0 as _, self.vector_table.start.0)
                .map_err(|e| RunnerError::UnableToReachMain(e))?;
        } else {
            // Reset the RTT control block
            core.write_word_32(self.symbols.rtt.0 as _, 0x12341234)
                .map_err(|e| RunnerError::UnableToReachMain(e))?;

            // Go to main
            core.set_hw_breakpoint(self.symbols.main.0 as _)
                .map_err(|e| RunnerError::UnableToReachMain(e))?;

            core.run().map_err(|e| RunnerError::UnableToReachMain(e))?;
            core.wait_for_core_halted(Duration::from_secs(5))
                .map_err(|e| RunnerError::UnableToReachMain(e))?;
            const OFFSET: u32 = 44;
            const FLAG: u32 = 2; // BLOCK_IF_FULL
            core.write_word_32((self.symbols.rtt.0 + OFFSET) as u64, FLAG)?;
            debug!("{}: Arrived at 'main'", self.probe_serial);
            core.clear_hw_breakpoint(self.symbols.main.0 as _)?;
        }

        if self.from_ram {
            // We can set breakpoints in RAM so we replace the instruction at the breakpoint
            // location with the breakpoint instruction instead.
            core.write_8(
                (self.vector_table.hardfault.0 as u32 & !THUMB_BIT) as u64,
                &[0x00, 0xbe],
            )?;
        } else {
            core.set_hw_breakpoint((self.vector_table.hardfault.0 as u32 & !THUMB_BIT) as u64)?;
        }

        core.run()?;

        // Attach to RTT.
        drop(core);
        let channel = self.setup_rtt_channel(&mut session)?;
        let mut core = session.core(0)?;

        let mut buffer = Vec::new();
        let mut read_buf = [0u8; 16 * 1024];
        let start = Instant::now();

        loop {
            // thread::sleep(Duration::from_millis(1));

            // Read from an RTT channel.
            let count = channel.read(&mut core, &mut read_buf[..])?;
            buffer.extend_from_slice(&read_buf[..count]);

            if core.core_halted()? {
                // Read from an RTT channel an extra time.
                let count = channel
                    .read(&mut core, &mut read_buf[..])
                    .map_err(|e| anyhow!(e))?;
                buffer.extend_from_slice(&read_buf[..count]);

                break;
            }

            if Instant::now() - start > timeout {
                let log = self.log_to_string(buffer).unwrap_or_default();
                debug!(
                    "{}: Firmware timeout, partial log:\n{}",
                    self.probe_serial, log
                );
                return Err(anyhow!(
                    "The firmware reached timeout, partial log:\n{}",
                    log
                ))?;
            }
        }

        let log = self.log_to_string(buffer)?;

        match core.status()? {
            CoreStatus::Halted(HaltReason::Breakpoint(_)) => {
                let isr_no = core.read_core_reg::<u32>(PSR)? & 0xff;

                if isr_no == 3 {
                    let return_address = core.read_core_reg::<u32>(LR)?;
                    let hfsr = core.read_word_32(0xE000_ED2C)?;

                    warn!("{}: Halted due to hardfault", self.probe_serial);
                    if hfsr & (1 << 30) != 0 {
                        let cfsr = core.read_word_32(0xE000_ED28)?;

                        let mut report = String::new();

                        let mmfsr = (cfsr & 0xff) as u8;
                        let bfsr = ((cfsr >> 8) & 0xff) as u8;
                        let ufsr = ((cfsr >> 16) & 0xffff) as u16;

                        report.push_str(&format!("  LR = {:#04x}\n", return_address));

                        if mmfsr != 0 {
                            report.push_str(&format!("  MemFault ({:#04x})\n", mmfsr));
                        }

                        if bfsr != 0 {
                            report.push_str(&format!("  BusFault ({:#04x})\n", bfsr));
                            if bfsr & 0x80 != 0 {
                                let bfar = core.read_word_32(0xE000_ED38)?;
                                report
                                    .push_str(&format!("    Offending address = {:#010x}\n", bfar));
                            }
                        }

                        if ufsr != 0 {
                            report.push_str(&format!("  UsageFault ({:#06x})\n", ufsr));
                        }

                        return Err(anyhow!(
                            "Core halted for hardfault\n{}\nPartial log:\n{}",
                            report,
                            log
                        )
                        .into());
                    }

                    return Err(anyhow!(
                        "Core halted for hardfault (LR = {:#010x}), partial log:\n{}",
                        return_address,
                        log
                    )
                    .into());
                } else {
                    debug!("{}: Halted due to breakpoint", self.probe_serial);
                }
            }
            CoreStatus::Halted(h) => {
                return Err(anyhow!("Core halted for unknown reason: {:?}", h).into());
            }
            CoreStatus::LockedUp => {
                return Err(anyhow!("Core locked up, partial log:\n{}", log).into());
            }
            CoreStatus::Sleeping => {
                return Err(anyhow!("Core sleeping, partial log:\n{}", log).into());
            }
            CoreStatus::Unknown => {
                return Err(anyhow!("Core status unknown, partial log:\n{}", log).into());
            }
            _ => {}
        }

        debug!(
            "{}: Log complete, size = {} bytes. Log:\n{}",
            self.probe_serial,
            log.len(),
            log
        );

        Ok(log)
    }

    /// Convert a raw log from a target to an actual readable format.
    fn log_to_string(&mut self, buffer: Vec<u8>) -> Result<String, RunnerError> {
        Ok(match &self.rtt_type {
            RttType::Defmt {
                table,
                _locations: _,
            } => {
                debug!(
                    "{}: Detected defmt log - decoding, buffer size = {} bytes",
                    self.probe_serial,
                    buffer.len()
                );

                let mut stream_decoder = table.new_stream_decoder();
                stream_decoder.received(&buffer);

                let mut log = String::new();

                loop {
                    match stream_decoder.decode() {
                        Ok(frame) => {
                            let level = match frame.level() {
                                Some(level) => format!("{:<5} ", level.as_str().to_uppercase()),
                                None => String::new(),
                            };

                            log.push_str(&format!("{}{}\n", level, frame.display_message()));
                        }
                        Err(DecodeError::Malformed) => {
                            if table.encoding().can_recover() {
                                continue;
                            } else {
                                warn!("{}: defmt stream is malformed, aborting", self.probe_serial);
                                return Ok(log);
                            }
                        }
                        Err(DecodeError::UnexpectedEof) => {
                            break;
                        }
                    }
                }

                log
            }
            RttType::PlainText => {
                debug!(
                    "{}: Plain-text log detected - decoding, buffer size = {} bytes",
                    self.probe_serial,
                    buffer.len()
                );

                String::from_utf8(buffer).map_err(|e| anyhow!(e))?
            }
        })
    }

    /// Helper function to set up RTT channels and compensate for common errors.
    fn setup_rtt_channel(&mut self, session: &mut Session) -> Result<UpChannel, RunnerError> {
        debug!("{}: Starting RTT pipe", self.probe_serial);
        let memory_map = session.target().memory_map.clone();
        let mut core = session.core(0)?;
        let start = Instant::now();

        let mut rtt = loop {
            match Rtt::attach_region(
                &mut core,
                &memory_map,
                &ScanRegion::Exact(self.symbols.rtt.0),
            ) {
                Ok(rtt) => break rtt,
                Err(RttError::ControlBlockNotFound) => {
                    thread::sleep(Duration::from_millis(10));
                    if Instant::now() - start > Duration::from_secs(3) {
                        return Err(RttError::ControlBlockNotFound.into());
                    }
                }
                Err(e) => return Err(e.into()),
            }
        };

        let channel = rtt
            .up_channels()
            .take(0)
            .ok_or(anyhow!("Could not open the RTT channel"))?;

        Ok(channel)
    }

    /// Get this runner's probe.
    fn get_probe(
        &self,
        probe_mutex: &Arc<Mutex<()>>,
        probe_speed_khz: Option<u32>,
    ) -> Result<Probe, RunnerError> {
        // Access to the list of probes needs to be unique, else the workers crash into each other.
        let guard = probe_mutex.lock().unwrap();
        let probe = {
            let all_probes = Probe::list_all();
            let mut probe = all_probes
                .iter()
                .find(|probe| {
                    if let Some(serial) = &probe.serial_number {
                        &self.probe_serial.0 == serial
                    } else {
                        false
                    }
                })
                .ok_or(DebugProbeError::ProbeCouldNotBeCreated(
                    ProbeCreationError::NotFound,
                ))?
                .open()?;

            if let Some(khz) = probe_speed_khz {
                if let Err(e) = probe.set_speed(khz) {
                    error!(
                        "{}; Unable to set probe speed, error: {}",
                        self.probe_serial,
                        unroll_error(&e)
                    );
                }
            }

            probe
        };
        drop(guard);

        Ok(probe)
    }
}
