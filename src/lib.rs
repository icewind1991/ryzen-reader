const AMD_TIME_UNIT_MASK: u64 = 0xF0000;
const AMD_ENERGY_UNIT_MASK: u64 = 0x1F00;
const AMD_POWER_UNIT_MASK: u64 = 0xF;

const MAX_CPUS: u32 = 1024;

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::mem::size_of;
use std::str;
use std::thread::sleep;
use std::time::Duration;
use thiserror::Error;

#[repr(u64)]
enum MsrValue {
    PowerUnit = 0xC0010299,
    CoreEnergy = 0xC001029A,
    PackageEnergy = 0xC001029B,
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("Permission denied when trying to open msr, are you running as root?")]
    PermissionDenied,
    #[error("Core with id not found")]
    CoreNotFound,
    #[error("IO error when trying to open msr: {0}")]
    IO(#[source] std::io::Error),
    #[error("No cores detected")]
    NoCores,
    #[error("Invalid package data")]
    InvalidPackage,
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            std::io::ErrorKind::NotFound => Self::CoreNotFound,
            _ => Error::IO(err),
        }
    }
}

struct Core {
    handle: RefCell<File>,
    package: u32,
}

impl Core {
    pub fn open(cpu_id: u32) -> Result<Self, Error> {
        let mut data = [0; 4];
        let mut package_handle = OpenOptions::new().read(true).open(&format!(
            "/sys/devices/system/cpu/cpu{}/topology/physical_package_id",
            cpu_id
        ))?;
        package_handle.read(&mut data)?;
        let package: u32 = str::from_utf8(&data)
            .map_err(|_| Error::InvalidPackage)?
            .trim_end_matches('\u{0}')
            .trim()
            .parse()
            .map_err(|_| Error::InvalidPackage)?;

        let handle = OpenOptions::new()
            .read(true)
            .write(false)
            .open(&format!("/dev/cpu/{}/msr", cpu_id))?;

        Ok(Core {
            handle: RefCell::new(handle),
            package,
        })
    }

    pub fn read(&self, value: MsrValue) -> Result<u64, Error> {
        let mut handle = self.handle.borrow_mut();
        handle.seek(SeekFrom::Start(value as u64))?;

        let mut data = [0; size_of::<u64>()];
        handle.read(&mut data)?;
        Ok(u64::from_le_bytes(data))
    }
}

#[derive(Debug, Clone)]
struct CorePower {
    core_power: f64,
    package_power: f64,
    package: u32,
}

#[derive(Debug, Clone)]
pub struct CpuPower {
    cores: Vec<CorePower>,
}

impl CpuPower {
    /// Get an iterator for all cpu cores in the system and their power draw in watt
    pub fn cores<'a>(&'a self) -> impl Iterator<Item = f64> + 'a {
        self.cores.iter().map(|core| core.core_power)
    }

    /// Get an iterator for all cpu packages in the system and their power draw in watt
    pub fn packages<'a>(&'a self) -> impl Iterator<Item = f64> + 'a {
        let mut last_package = u32::max_value();

        let mut packages = Vec::new();

        for core in self.cores.iter() {
            if core.package != last_package {
                last_package = core.package;
                packages.push(core.package_power)
            }
        }

        packages.into_iter()
    }
}

#[derive(Debug)]
struct PowerUnits {
    time_unit: f64,
    energy_unit: f64,
    power_unit: f64,
}

pub struct CpuInfo {
    cores: Vec<Core>,
    units: PowerUnits,
}

/// Struct that allows reading of cpu power info
///
/// # Example
///
/// ```rust
/// # use ryzen_reader::{CpuInfo, Error};
/// #
/// # fn main() -> Result<(), Error> {
///     let cpu = CpuInfo::new()?;
///     let power = cpu.read()?;
///
///     println!("Package power:");
///     for (package, usage) in power.packages().enumerate() {
///         println!("\t#{}: {:.2}W", package, usage);
///     }
///     println!("Core power:");
///     for (core, usage) in power.cores().enumerate() {
///         println!("\t#{}: {:.2}W", core, usage);
///     }
/// #     Ok(())
/// # }
///```
impl CpuInfo {
    pub fn new() -> Result<Self, Error> {
        let mut cores = Vec::with_capacity(8);

        for i in 0..MAX_CPUS {
            match Core::open(i) {
                Ok(core) => cores.push(core),
                Err(Error::CoreNotFound) => break,
                Err(e) => return Err(e),
            }
        }

        if cores.is_empty() {
            return Err(Error::NoCores);
        }

        let units = cores[0].read(MsrValue::PowerUnit)?;
        let time_unit = (units & AMD_TIME_UNIT_MASK) >> 16;
        let energy_unit = (units & AMD_ENERGY_UNIT_MASK) >> 8;
        let power_unit = units & AMD_POWER_UNIT_MASK;

        let time_unit = 0.5f64.powi(time_unit as i32);
        let energy_unit = 0.5f64.powi(energy_unit as i32);
        let power_unit = 0.5f64.powi(power_unit as i32);

        let units = PowerUnits {
            time_unit,
            energy_unit,
            power_unit,
        };

        Ok(CpuInfo { cores, units })
    }

    /// Read the cpu power levels
    ///
    /// Note that this method will block for ~10ms
    pub fn read(&self) -> Result<CpuPower, Error> {
        let start = self.read_raw()?;
        sleep(Duration::from_millis(10));
        let end = self.read_raw()?;

        let cores = start
            .into_iter()
            .zip(end.into_iter())
            .map(|(start, end)| CorePower {
                core_power: (end.core_power - start.core_power) * 100.0,
                package_power: (end.package_power - start.package_power) * 100.0,
                package: start.package,
            })
            .collect();

        Ok(CpuPower { cores })
    }

    fn read_raw(&self) -> Result<Vec<CorePower>, Error> {
        self.cores
            .iter()
            .map(|core| {
                let core_power = core.read(MsrValue::CoreEnergy)? as f64 * self.units.energy_unit;
                let package_power =
                    core.read(MsrValue::PackageEnergy)? as f64 * self.units.energy_unit;
                Ok(CorePower {
                    core_power,
                    package_power,
                    package: core.package,
                })
            })
            .collect()
    }
}
