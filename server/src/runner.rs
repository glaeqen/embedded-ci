use anyhow::anyhow;
use defmt_decoder::{DecodeError, Locations as DefmtLocations, Table as DefmtTable};
use log::*;
use object::{File, Object, ObjectSection, ObjectSymbol};
use probe_rs::{flashing::DownloadOptions, CoreRegisterAddress, MemoryInterface, Session};
use probe_rs::{CoreStatus, DebugProbeError, HaltReason, Probe, ProbeCreationError};
use probe_rs_rtt::{Rtt, ScanRegion, UpChannel};
use std::io::Cursor;
use std::thread;
use std::time::{Duration, Instant};

const THUMB_BIT: u32 = 1;
pub const LR: CoreRegisterAddress = CoreRegisterAddress(14);
pub const PC: CoreRegisterAddress = CoreRegisterAddress(15);
pub const SP: CoreRegisterAddress = CoreRegisterAddress(13);
pub const XPSR: CoreRegisterAddress = CoreRegisterAddress(16);
pub const VTOR: Address = Address(0xE000ED08);

pub struct Address(pub u32);

enum RttType {
    Defmt {
        table: DefmtTable,
        locations: DefmtLocations,
    },
    PlainText,
}

pub struct Runner<'a> {
    target_name: &'a str,
    probe_serial: &'a str,
    from_ram: bool,
    symbols: Symbols,
    vector_table: VectorTable,
    rtt_type: RttType,
    elf_bytes: &'a [u8],
}

pub struct Symbols {
    main: Address,
    rtt: Address,
}

pub struct VectorTable {
    start: Address,
    stack_pointer: Address,
    reset: Address,
    hardfault: Address,
}
impl<'a> Runner<'a> {
    pub fn new(
        elf_bytes: &'a [u8],
        target_name: &'a str,
        probe_serial: &'a str,
    ) -> anyhow::Result<Runner<'a>> {
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
                    return Err(anyhow!("Section '{}' is not 4 byte aligned", name));
                }

                // If it is the vector table, get important addresses from it
                if name == ".vector_table" {
                    let data = section.data()?;
                    if data.len() < 16 {
                        return Err(anyhow!(
                            "Section '{}' is too small, size = {} bytes",
                            name,
                            data.len()
                        ));
                    }

                    let vt: Vec<_> = data
                        .chunks_exact(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect();

                    vector_table = Some(VectorTable {
                        start: Address(addr.try_into()?),
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

            if !table.is_empty() && locations.is_empty() {
                return Err(anyhow!(
                    "'.defmt' symbol found but not enough debug information for defmt, enable debug symbols (debug = 2)"
                ));
            } else {
                RttType::Defmt { table, locations }
            }
        } else {
            // The defmt table parsing returned none, so there is no `.defmt` section
            RttType::PlainText
        };

        Ok(Runner {
            target_name,
            probe_serial,
            from_ram,
            symbols,
            vector_table: vector_table.ok_or(anyhow!("'.vector_table' section not found"))?,
            rtt_type,
            elf_bytes,
        })
    }

    pub fn run(&mut self, timeout: Duration) -> anyhow::Result<String> {
        let start = Instant::now();
        let probe = self.get_probe()?;

        debug!("{}: Attaching to target", self.probe_serial);
        let mut session = probe.attach_under_reset(self.target_name)?;

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
        core.reset_and_halt(Duration::from_secs(3))?;

        // Check so we have some breakpoint units
        if core.get_available_breakpoint_units()? == 0 {
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
            core.write_core_reg(PC, self.vector_table.reset.0)?;
            core.write_core_reg(SP, self.vector_table.stack_pointer.0)?;
            core.write_word_32(VTOR.0, self.vector_table.start.0)?;
        } else {
            // Corrupt the rtt control block so that it's setup fresh again
            // Only do this when running from flash, because when running from RAM the
            // "fake-flashing to RAM" is what initializes it.
            core.write_word_32(self.symbols.rtt.0, 0xdeadc0de)?;

            core.set_hw_breakpoint(self.symbols.main.0)?;
            core.run()?;
            core.wait_for_core_halted(Duration::from_secs(5))?;
            // const OFFSET: u32 = 44;
            // const FLAG: u32 = 2; // BLOCK_IF_FULL
            // core.write_word_32(self.symbols.rtt.0 + OFFSET, FLAG)?;
            debug!("{}: Arrived at 'main'", self.probe_serial);
            core.clear_hw_breakpoint(self.symbols.main.0)?;
        }

        core.run()?;

        // Attach to RTT.
        drop(core);
        let channel = self.setup_rtt_channel(&mut session)?;
        let mut core = session.core(0)?;

        let mut buffer = Vec::new();
        let mut read_buf = [0u8; 16 * 1024];

        loop {
            // thread::sleep(Duration::from_millis(1));

            // Read from an RTT channel.
            let count = channel
                .read(&mut core, &mut read_buf[..])
                .map_err(|e| anyhow!(e))?;
            buffer.extend_from_slice(&read_buf[..count]);

            match core.status()? {
                CoreStatus::Halted(HaltReason::Breakpoint) => {
                    debug!("{}: Halted due to breakpoint", self.probe_serial);
                    break;
                }
                CoreStatus::Halted(h) => {
                    // TODO: handle hardfault
                    return Err(anyhow!("Core halted for unknown reason: {:?}", h));
                }
                _ => {}
            }

            if Instant::now() - start > timeout {
                debug!("{}: Firmware timeout", self.probe_serial);
                break;
                // return Err(anyhow!("The firmware reached timeout"))?;
            }
        }

        let log = self.log_to_string(buffer)?;

        debug!(
            "{}: Log complete, size = {} bytes. Log: {}",
            self.probe_serial,
            log.len(),
            log
        );

        Ok(log)
    }

    fn log_to_string(&mut self, buffer: Vec<u8>) -> anyhow::Result<String> {
        Ok(match &self.rtt_type {
            RttType::Defmt {
                table,
                locations: _,
            } => {
                debug!(
                    "{}: Decoding defmt log, buffer size = {} bytes",
                    self.probe_serial,
                    buffer.len()
                );

                let mut stream_decoder = table.new_stream_decoder();
                stream_decoder.received(&buffer);

                let mut log = String::new();

                loop {
                    match stream_decoder.decode() {
                        Ok(frame) => {
                            log.push_str(&format!("{}", frame.display_message()));
                        }
                        Err(DecodeError::Malformed) => {
                            if table.encoding().can_recover() {
                                continue;
                            } else {
                                return Err(anyhow!("defmt stream is malformed, aborting"));
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
                    "{}: Decoding plain-text log, buffer size = {} bytes",
                    self.probe_serial,
                    buffer.len()
                );

                String::from_utf8(buffer).map_err(|e| anyhow!(e))?
            }
        })
    }

    fn setup_rtt_channel(&mut self, session: &mut Session) -> anyhow::Result<UpChannel> {
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
                Err(probe_rs_rtt::Error::ControlBlockNotFound) => {
                    thread::sleep(Duration::from_millis(10));
                    if Instant::now() - start > Duration::from_secs(3) {
                        return Err(anyhow!("Unable to attach to RTT: 'ControlBlockNotFound'"));
                    }
                }
                Err(e) => return Err(anyhow!(e)),
            }
        };

        let channel = rtt
            .up_channels()
            .take(0)
            .ok_or(anyhow!("Could not open the RTT channel"))?;

        Ok(channel)
    }

    fn get_probe(&self) -> anyhow::Result<Probe> {
        let all_probes = Probe::list_all();
        let probe = all_probes
            .iter()
            .find(|probe| {
                if let Some(serial) = &probe.serial_number {
                    self.probe_serial == serial
                } else {
                    false
                }
            })
            .ok_or(DebugProbeError::ProbeCouldNotBeCreated(
                ProbeCreationError::NotFound,
            ))?
            .open()?;

        Ok(probe)
    }
}
