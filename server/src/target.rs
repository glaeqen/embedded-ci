use embedded_ci_server::{CpuId, ProbeSerial};
use log::*;
use num_enum::TryFromPrimitive;
use probe_rs::{MemoryInterface, Probe, WireProtocol};
use std::collections::HashMap;
use std::time::Duration;

macro_rules! skip_fail {
    ($res:expr) => {
        match $res {
            Ok(val) => val,
            Err(e) => {
                warn!("An error in probe & target detection: {}; skipped.", e);
                continue;
            }
        }
    };
}

/// Get all conencted MCUs.
pub fn get_mcus() -> HashMap<ProbeSerial, CpuId> {
    let probes: Vec<_> = Probe::list_all()
        .iter()
        .cloned()
        .filter(|probe| {
            if probe.serial_number.is_none() {
                warn!(
                    "Probe with VID = 0x{:x}, PID = 0x{:x} does not have a serial number and will not be used by this service", 
                    probe.vendor_id, probe.product_id
                );
            }

            probe.serial_number.is_some()
        })
        .collect();

    let mut mcus = HashMap::new();

    for probe in probes {
        let serial = probe.serial_number.clone().unwrap();

        let mut probe = skip_fail!(probe.open());
        // skip_fail!(probe.target_reset());
        skip_fail!(probe.select_protocol(WireProtocol::Swd));

        let mut session = skip_fail!(probe.attach("armv6m"));
        let mut core = skip_fail!(session.core(0));
        skip_fail!(core.halt(Duration::from_secs(3)));

        let value = skip_fail!(core.read_word_32(0xE000ED00));
        let cpuid_val = (value >> 4) & 0xfff;
        let cpuid = skip_fail!(CpuId::try_from_primitive(cpuid_val));

        mcus.insert(ProbeSerial(serial), cpuid);
    }

    mcus
}
