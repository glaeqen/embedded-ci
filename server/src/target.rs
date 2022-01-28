use log::*;
use num_enum::TryFromPrimitive;
use probe_rs::{DebugProbeInfo, MemoryInterface, Probe, WireProtocol};
use rocket::request::FromParam;
use rocket_okapi::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(
    Debug, Clone, Copy, TryFromPrimitive, Hash, PartialEq, Eq, JsonSchema, Serialize, Deserialize,
)]
#[repr(u32)]
pub enum CpuId {
    CortexM0 = 0xc20,
    CortexM0Plus = 0xc60,
    CortexM1 = 0xc21,
    CortexM3 = 0xc23,
    CortexM4 = 0xc24,
    CortexM7 = 0xc27,
    CortexM23 = 0xd20,
    CortexM33 = 0xd21,
}

impl FromParam<'_> for CpuId {
    type Error = ();

    fn from_param(param: &str) -> Result<Self, Self::Error> {
        Ok(match param {
            "CortexM0" => CpuId::CortexM0,
            "CortexM0Plus" => CpuId::CortexM0Plus,
            "CortexM1" => CpuId::CortexM1,
            "CortexM3" => CpuId::CortexM3,
            "CortexM4" => CpuId::CortexM4,
            "CortexM7" => CpuId::CortexM7,
            "CortexM23" => CpuId::CortexM23,
            "CortexM33" => CpuId::CortexM33,
            _ => return Err(()),
        })
    }
}

macro_rules! skip_fail {
    ($res:expr) => {
        match $res {
            Ok(val) => val,
            Err(e) => {
                warn!("An error: {}; skipped.", e);
                continue;
            }
        }
    };
}

fn get_mcus() -> HashMap<String, CpuId> {
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
        let serial = probe.serial_number.clone();
        let mut probe = skip_fail!(probe.open());
        skip_fail!(probe.target_reset());
        skip_fail!(probe.select_protocol(WireProtocol::Swd));

        let mut session = skip_fail!(probe.attach("armv6m"));
        let mut core = skip_fail!(session.core(0));
        skip_fail!(core.halt(Duration::from_secs(3)));

        let value = skip_fail!(core.read_word_32(0xE000ED00));
        let cpuid_val = (value >> 4) & 0xfff;
        let cpuid = skip_fail!(CpuId::try_from_primitive(cpuid_val));

        if let Some(serial) = serial {
            mcus.insert(serial, cpuid);
        }
    }

    mcus
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetSettings {
    pub probe_serial: String,
    pub target_name: String,
}

#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub cpu_type: CpuId,
    pub probe_serial: String,
    pub target_name: String,
}

#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Targets {
    targets: Vec<Target>,
}

impl Targets {
    pub fn from_target_settings(target_settings: &[TargetSettings]) -> Self {
        let mut attached_targets = get_mcus();
        let mut targets = Vec::new();

        for setting in target_settings {
            if let Some((probe_serial, cpu_type)) =
                attached_targets.remove_entry(&setting.probe_serial)
            {
                targets.push(Target {
                    cpu_type,
                    probe_serial,
                    target_name: setting.target_name.clone(),
                });
            } else {
                warn!(
                    "Probe with serial '{}' is not attached.",
                    setting.probe_serial
                );
            }
        }

        for (ps, _) in attached_targets {
            warn!("Probe with serial '{}' does not have a configuration.", ps);
        }

        Targets { targets }
    }

    pub fn get_core(&self, core: &CpuId) -> Option<&Target> {
        self.targets.iter().find(|target| &target.cpu_type == core)
    }

    pub fn get_probe(&self, probe_serial: &str) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.probe_serial == probe_serial)
    }

    pub fn get_target(&self, target_name: &str) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| &target.target_name == target_name)
    }

    pub fn all_targets(&self) -> &[Target] {
        &self.targets
    }
}
