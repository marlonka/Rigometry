//! Optional NVIDIA driver telemetry. Only absolute driver installation paths are loaded.
// ABI declarations follow NVIDIA's public NVML interface. No driver is redistributed.
// Copyright 1993-2026 NVIDIA Corporation. All rights reserved.
// NVIDIA MAKES NO REPRESENTATION ABOUT THE SUITABILITY OF THIS SOURCE CODE FOR
// ANY PURPOSE. IT IS PROVIDED "AS IS" WITHOUT EXPRESS OR IMPLIED WARRANTY OF
// ANY KIND. NVIDIA DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOURCE CODE,
// INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY, NONINFRINGEMENT, AND
// FITNESS FOR A PARTICULAR PURPOSE. IN NO EVENT SHALL NVIDIA BE LIABLE FOR ANY
// SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL DAMAGES, OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOURCE CODE.
// U.S. Government End Users. This source code is a "commercial item" as that
// term is defined at 48 C.F.R. 2.101 (OCT 1995), consisting of "commercial
// computer software" and "commercial computer software documentation" as such
// terms are used in 48 C.F.R. 12.212 (SEPT 1995) and is provided to the U.S.
// Government only as a commercial end item. Consistent with 48 C.F.R.12.212
// and 48 C.F.R. 227.7202-1 through 227.7202-4 (JUNE 1995), all U.S. Government
// End Users acquire the source code with only those rights set forth herein.

use crate::model::{Adapter, Availability, Field, Reading, bytes};
use ::windows::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        System::{
            Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
            SystemInformation::GetSystemDirectoryW,
        },
    },
    core::w,
};
use libloading::Library;
use libloading::os::windows::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library as WindowsLibrary,
};
use std::{
    ffi::{OsString, c_char, c_void},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
};

type Handle = *mut c_void;
type Status = u32;
type GetU32 = unsafe extern "C" fn(Handle, *mut u32) -> Status;
type GetSelectorU32 = unsafe extern "C" fn(Handle, u32, *mut u32) -> Status;
#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}
#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}
#[repr(C)]
#[derive(Default)]
struct PciInfo {
    legacy: [c_char; 16],
    domain: u32,
    bus: u32,
    device: u32,
    device_id: u32,
    subsystem: u32,
    id: [c_char; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    bus: u32,
    device: u32,
    function: u32,
    vendor: u32,
    device_id: u32,
}
struct Device {
    handle: Handle,
    identity: Identity,
}
pub(super) struct Nvml {
    library: Library,
    devices: Vec<Device>,
}

impl Nvml {
    pub(super) fn load() -> Result<Self, String> {
        Self::load_from_paths(driver_library_paths()?)
    }
    fn load_from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let mut errors = Vec::new();
        for path in paths {
            if !path.is_absolute() || !path.exists() {
                continue;
            }
            // Restrict dependency resolution as well as the top-level DLL.
            // The application directory, current directory, PATH and user DLL
            // search directories cannot supply dependencies for this load.
            match unsafe {
                WindowsLibrary::load_with_flags(
                    &path,
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            } {
                Ok(library) => return Self::initialize(library.into()),
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
        Err(if errors.is_empty() {
            "nvml.dll not found in System32 or NVIDIA Corporation/NVSMI. Install a supported NVIDIA display driver to enable vendor sensors.".into()
        } else {
            errors.join("; ")
        })
    }
    fn initialize(library: Library) -> Result<Self, String> {
        unsafe {
            // Require the cleanup entry point before any initialization side
            // effects. Nvml::drop runs while the owning library is still live.
            let _ = library
                .get::<unsafe extern "C" fn() -> Status>(b"nvmlShutdown\0")
                .map_err(|e| e.to_string())?;
            let init = library
                .get::<unsafe extern "C" fn() -> Status>(b"nvmlInit_v2\0")
                .map_err(|e| e.to_string())?;
            let status = init();
            if status != 0 {
                return Err(error(status));
            }
            let mut result = Self {
                library,
                devices: Vec::new(),
            };
            let count_fn = result
                .library
                .get::<unsafe extern "C" fn(*mut u32) -> Status>(b"nvmlDeviceGetCount_v2\0")
                .map_err(|e| e.to_string())?;
            let handle_fn = result
                .library
                .get::<unsafe extern "C" fn(u32, *mut Handle) -> Status>(
                    b"nvmlDeviceGetHandleByIndex_v2\0",
                )
                .map_err(|e| e.to_string())?;
            let pci_fn = result
                .library
                .get::<unsafe extern "C" fn(Handle, *mut PciInfo) -> Status>(
                    b"nvmlDeviceGetPciInfo_v3\0",
                )
                .map_err(|e| e.to_string())?;
            let mut count = 0;
            let status = count_fn(&mut count);
            if status != 0 {
                return Err(error(status));
            }
            result.devices = enumerate_devices(count, |index| {
                let mut handle = std::ptr::null_mut();
                let status = handle_fn(index, &mut handle);
                if status != 0 {
                    return Err(error(status));
                }
                if handle.is_null() {
                    return Err("NVML returned a null device handle".into());
                }
                let mut pci = PciInfo::default();
                let status = pci_fn(handle, &mut pci);
                if status != 0 {
                    return Err(error(status));
                }
                let pci_string = chars(&pci.id);
                let (domain, bus, device, function) =
                    parse_pci(&pci_string).ok_or("NVML returned a malformed PCI identifier")?;
                if (domain, bus, device) != (pci.domain, pci.bus, pci.device) {
                    return Err("NVML PCI text and numeric identity disagree".into());
                }
                Ok(Device {
                    handle,
                    identity: Identity {
                        bus,
                        device,
                        function,
                        vendor: pci.device_id & 0xffff,
                        device_id: pci.device_id >> 16,
                    },
                })
            })?;
            let mut after = 0;
            let status = count_fn(&mut after);
            if status != 0 || after != count {
                return Err(
                    "NVML device topology changed during enumeration; association refused".into(),
                );
            }
            Ok(result)
        }
    }
    fn device(&self, adapter: &Adapter) -> Result<Handle, String> {
        let address = adapter
            .fields
            .iter()
            .find(|f| f.label == "PCI address")
            .and_then(|f| f.value.as_deref())
            .ok_or(
                "No Windows PCI address; refusing to match sensors by name or enumeration index",
            )?;
        let (_, bus, device, function) =
            parse_pci(&format!("0000:{address}")).ok_or("Invalid Windows PCI address")?;
        let wanted = Identity {
            bus,
            device,
            function,
            vendor: adapter.vendor_id,
            device_id: adapter.device_id,
        };
        match_handle(&self.devices, &wanted).map_err(|e| {
            format!(
                "PCI {address}, {:04X}:{:04X}: {e}",
                adapter.vendor_id, adapter.device_id
            )
        })
    }
    fn number(&self, handle: Handle, name: &[u8]) -> Result<u32, Status> {
        unsafe {
            let call = self.library.get::<GetU32>(name).map_err(|_| 3u32)?;
            let mut value = 0;
            let status = call(handle, &mut value);
            if status == 0 { Ok(value) } else { Err(status) }
        }
    }
    fn selected(&self, handle: Handle, name: &[u8], selector: u32) -> Result<u32, Status> {
        unsafe {
            let call = self.library.get::<GetSelectorU32>(name).map_err(|_| 3u32)?;
            let mut value = 0;
            let status = call(handle, selector, &mut value);
            if status == 0 { Ok(value) } else { Err(status) }
        }
    }
    fn string(&self, handle: Handle, name: &[u8]) -> Result<String, Status> {
        unsafe {
            let call = self
                .library
                .get::<unsafe extern "C" fn(Handle, *mut c_char, u32) -> Status>(name)
                .map_err(|_| 3u32)?;
            let mut data = [0 as c_char; 128];
            let status = call(handle, data.as_mut_ptr(), 128);
            if status == 0 {
                Ok(chars(&data))
            } else {
                Err(status)
            }
        }
    }
    fn memory(&self, handle: Handle) -> Result<Memory, Status> {
        unsafe {
            self.library
                .get::<unsafe extern "C" fn(Handle, *mut Memory) -> Status>(
                    b"nvmlDeviceGetMemoryInfo\0",
                )
                .map_err(|_| 13u32)
                .and_then(|call| {
                    let mut value = Memory::default();
                    let status = call(handle, &mut value);
                    if status == 0 { Ok(value) } else { Err(status) }
                })
        }
    }
    pub(super) fn describe(&self, adapter: &mut Adapter) {
        let handle = match self.device(adapter) {
            Ok(h) => h,
            Err(e) => {
                adapter.fields.push(Field::missing(
                    "NVIDIA sensors",
                    Availability::Unavailable,
                    "NVML PCI identity",
                    e,
                ));
                return;
            }
        };
        adapter.fields.push(Field::valid(
            "Sensor binding",
            "Unique PCI bus/device/function + vendor/device match",
            "",
            "D3DKMT + NVML",
        ));
        match self.memory(handle) {
            Ok(memory)=>adapter.fields.push(Field::valid("Physical device memory capacity",bytes(memory.total),"","NVML nvmlDeviceGetMemoryInfo.total; physical device memory, distinct from DXGI graphics capacity")),
            Err(status)=>adapter.fields.push(Field::missing("Physical device memory capacity",state(status),"NVML nvmlDeviceGetMemoryInfo.total",error(status))),
        }
        for (label, function, unit) in [
            (
                "Maximum PCIe generation",
                b"nvmlDeviceGetMaxPcieLinkGeneration\0".as_slice(),
                "",
            ),
            (
                "Maximum PCIe width",
                b"nvmlDeviceGetMaxPcieLinkWidth\0".as_slice(),
                "lanes",
            ),
            (
                "GPU core count",
                b"nvmlDeviceGetNumGpuCores\0".as_slice(),
                "",
            ),
            (
                "Memory bus width",
                b"nvmlDeviceGetMemoryBusWidth\0".as_slice(),
                "bits",
            ),
        ] {
            match self.number(handle, function) {
                Ok(v) => adapter.fields.push(Field::valid(
                    label,
                    v.to_string(),
                    unit,
                    "NVML / NVIDIA driver",
                )),
                Err(status) => adapter.fields.push(Field::missing(
                    label,
                    state(status),
                    "NVML / NVIDIA driver",
                    error(status),
                )),
            }
        }
        if let Ok(vbios) = self.string(handle, b"nvmlDeviceGetVbiosVersion\0") {
            adapter
                .fields
                .push(Field::valid("NVIDIA video BIOS", vbios, "", "NVML"));
        }
    }
    pub(super) fn sample(&self, adapter: &Adapter, readings: &mut Vec<(String, Reading)>) {
        let handle = match self.device(adapter) {
            Ok(h) => h,
            Err(e) => {
                readings.push((
                    "NVIDIA provider".into(),
                    Reading::missing("", "NVML PCI identity", &e),
                ));
                return;
            }
        };
        let utilization = unsafe {
            self.library
                .get::<unsafe extern "C" fn(Handle, *mut Utilization) -> Status>(
                    b"nvmlDeviceGetUtilizationRates\0",
                )
                .map_err(|_| 3u32)
                .and_then(|call| {
                    let mut value = Utilization::default();
                    let status = call(handle, &mut value);
                    if status == 0 { Ok(value) } else { Err(status) }
                })
        };
        for (label, unit, value) in [
            (
                "Utilization",
                "%",
                utilization.as_ref().map(|v| v.gpu as f64).map_err(|s| *s),
            ),
            (
                "Memory controller load",
                "%",
                utilization
                    .as_ref()
                    .map(|v| v.memory as f64)
                    .map_err(|s| *s),
            ),
            (
                "Temperature",
                "°C",
                self.selected(handle, b"nvmlDeviceGetTemperature\0", 0)
                    .map(|v| v as f64),
            ),
            (
                "Graphics clock",
                "MHz",
                self.selected(handle, b"nvmlDeviceGetClockInfo\0", 0)
                    .map(|v| v as f64),
            ),
            (
                "Memory clock",
                "MHz",
                self.selected(handle, b"nvmlDeviceGetClockInfo\0", 2)
                    .map(|v| v as f64),
            ),
            (
                "Power",
                "W",
                self.number(handle, b"nvmlDeviceGetPowerUsage\0")
                    .map(|v| v as f64 / 1000.0),
            ),
            (
                "Fan speed",
                "%",
                self.number(handle, b"nvmlDeviceGetFanSpeed\0")
                    .map(|v| v as f64),
            ),
            (
                "PCIe generation",
                "",
                self.number(handle, b"nvmlDeviceGetCurrPcieLinkGeneration\0")
                    .map(|v| v as f64),
            ),
            (
                "PCIe width",
                "lanes",
                self.number(handle, b"nvmlDeviceGetCurrPcieLinkWidth\0")
                    .map(|v| v as f64),
            ),
        ] {
            merge(readings, label, reading(value, unit));
        }
        // Retain Windows dedicated usage as a distinct definition.
        readings.push((
            "NVIDIA memory allocated + reserved".into(),
            memory_usage(self.memory(handle)),
        ));
    }
}
impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            if let Ok(shutdown) = self
                .library
                .get::<unsafe extern "C" fn() -> Status>(b"nvmlShutdown\0")
            {
                let _ = shutdown();
            }
        }
    }
}

fn driver_library_paths() -> Result<Vec<PathBuf>, String> {
    let mut buffer = vec![0u16; 32768];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if len == 0 || len >= buffer.len() {
        return Err(format!(
            "Windows system directory unavailable: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut paths = vec![PathBuf::from(OsString::from_wide(&buffer[..len])).join("nvml.dll")];
    // Legacy NVSMI location comes from machine configuration, not overridable
    // process environment variables. Normal user permissions cannot rewrite it.
    unsafe {
        let mut bytes = 0;
        let key = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion");
        let name = w!("ProgramFilesDir");
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key,
            name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        ) == ERROR_SUCCESS
            && bytes > 2
            && bytes <= 65536
        {
            let mut root = vec![0u16; (bytes as usize).div_ceil(2)];
            if RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(root.as_mut_ptr().cast()),
                Some(&mut bytes),
            ) == ERROR_SUCCESS
            {
                let length = root.iter().position(|v| *v == 0).unwrap_or(root.len());
                let directory = PathBuf::from(OsString::from_wide(&root[..length]));
                if directory.is_absolute() {
                    paths.push(directory.join("NVIDIA Corporation/NVSMI/nvml.dll"));
                }
            }
        }
    }
    Ok(paths)
}

fn chars(value: &[c_char]) -> String {
    String::from_utf8_lossy(
        &value
            .iter()
            .take_while(|v| **v != 0)
            .map(|v| *v as u8)
            .collect::<Vec<_>>(),
    )
    .into_owned()
}
fn match_handle(devices: &[Device], wanted: &Identity) -> Result<Handle, String> {
    let mut candidates = devices.iter().filter(|d| &d.identity == wanted);
    let first = candidates.next().ok_or(
        "No NVML device matches Windows PCI bus/device/function and vendor/device identity",
    )?;
    if candidates.next().is_some() {
        return Err("PCI identity is ambiguous across domains; sensor association refused".into());
    }
    Ok(first.handle)
}

fn enumerate_devices(
    count: u32,
    mut read: impl FnMut(u32) -> Result<Device, String>,
) -> Result<Vec<Device>, String> {
    if count > 64 {
        return Err(
            "NVML device count exceeds 64; partial enumeration cannot prove a unique PCI match"
                .into(),
        );
    }
    (0..count)
        .map(|index| {
            read(index).map_err(|error| {
                format!("NVML device {index} identification failed: {error}; incomplete enumeration cannot prove a unique PCI match")
            })
        })
        .collect()
}
fn parse_pci(value: &str) -> Option<(u32, u32, u32, u32)> {
    let mut p = value.split(':');
    let domain = u32::from_str_radix(p.next()?, 16).ok()?;
    let bus = u32::from_str_radix(p.next()?, 16).ok()?;
    let (device, function) = p.next()?.split_once('.')?;
    let device = u32::from_str_radix(device, 16).ok()?;
    let function = u32::from_str_radix(function, 16).ok()?;
    if p.next().is_some() || bus > 255 || device > 31 || function > 7 {
        return None;
    }
    Some((domain, bus, device, function))
}
fn state(status: Status) -> Availability {
    match status {
        3 | 13 => Availability::Unsupported,
        4 | 17 => Availability::PermissionRequired,
        6 | 9 | 12 | 15 | 21 | 27 | 28 => Availability::Unavailable,
        _ => Availability::Failed,
    }
}
fn error(status: Status) -> String {
    format!(
        "NVML status {status}: {}",
        match status {
            1 => "not initialized",
            2 => "invalid argument",
            3 => "not supported by this GPU/driver or API symbol absent",
            4 => "permission required",
            6 => "object not found",
            9 => "NVIDIA driver not loaded",
            12 => "library unavailable",
            13 => "function not implemented by the installed driver",
            15 => "GPU lost",
            17 => "GPU access blocked by the operating system",
            18 => "driver/library version mismatch",
            21 => "no data",
            27 => "provider not ready",
            28 => "GPU not found",
            _ => "provider call failed",
        }
    )
}
fn reading(value: Result<f64, Status>, unit: &str) -> Reading {
    match value {
        Ok(v) => Reading::valid(v, unit, "NVML / NVIDIA display driver"),
        Err(status) => {
            let mut r = Reading::missing(unit, "NVML / NVIDIA display driver", &error(status));
            r.state = state(status);
            r
        }
    }
}
fn memory_usage(memory: Result<Memory, Status>) -> Reading {
    let mut used = reading(memory.map(|memory| memory.used as f64), "bytes");
    if used.state == Availability::Valid {
        used.detail = "NVML v1 used memory includes driver-reserved and allocated memory; Windows WDDM may not expose this metric.".into();
    }
    used
}
fn merge(readings: &mut Vec<(String, Reading)>, label: &str, value: Reading) {
    if let Some((_, current)) = readings.iter_mut().find(|(name, _)| name == label) {
        if value.state == Availability::Valid || current.state != Availability::Valid {
            *current = value;
        }
    } else {
        readings.push((label.into(), value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_device_enumeration_cannot_establish_unique_identity() {
        let mut called = false;
        assert!(
            enumerate_devices(65, |_| {
                called = true;
                unreachable!()
            })
            .is_err()
        );
        assert!(
            !called,
            "Oversized enumeration must be rejected before querying devices"
        );
        let result = enumerate_devices(2, |index| {
            if index == 1 {
                return Err("Device disappeared".into());
            }
            Ok(Device {
                handle: 1usize as Handle,
                identity: Identity {
                    bus: 1,
                    device: 0,
                    function: 0,
                    vendor: 0x10de,
                    device_id: 1,
                },
            })
        });
        assert!(matches!(result, Err(ref error) if error.contains("incomplete enumeration")));
    }
    #[test]
    fn absent_driver_library_is_a_recoverable_provider_error() {
        let directory = tempfile::tempdir().unwrap();
        let absent = directory.path().join("nvml.dll");
        assert!(absent.is_absolute());
        let result = Nvml::load_from_paths([absent]);
        assert!(matches!(result,Err(ref e) if e.contains("nvml.dll not found")));
    }
    #[test]
    fn reordered_and_ambiguous_identical_gpu_models_never_bind_by_index() {
        let wanted = Identity {
            bus: 9,
            device: 0,
            function: 0,
            vendor: 0x10de,
            device_id: 0x2c05,
        };
        let other = Identity {
            bus: 1,
            ..wanted.clone()
        };
        let mut devices = vec![
            Device {
                handle: 1usize as Handle,
                identity: other,
            },
            Device {
                handle: 2usize as Handle,
                identity: wanted.clone(),
            },
        ];
        assert_eq!(match_handle(&devices, &wanted).unwrap(), 2usize as Handle);
        devices.reverse();
        assert_eq!(match_handle(&devices, &wanted).unwrap(), 2usize as Handle);
        devices.push(Device {
            handle: 3usize as Handle,
            identity: wanted.clone(),
        });
        assert!(match_handle(&devices, &wanted).is_err());
        let wrong_function = Identity {
            function: 1,
            ..wanted
        };
        assert!(match_handle(&devices, &wrong_function).is_err());
    }
    #[test]
    fn pci_domains_functions_and_bounds_are_not_discarded() {
        assert_eq!(parse_pci("00000001:2A:03.1"), Some((1, 42, 3, 1)));
        assert_eq!(parse_pci("0000:2A:03.0"), Some((0, 42, 3, 0)));
        assert!(parse_pci("0000:100:00.0").is_none());
        assert!(parse_pci("0000:2A:03.8").is_none());
    }
    #[test]
    fn zero_vendor_reading_replaces_fallback_and_error_does_not() {
        let mut values = vec![("Temperature".into(), Reading::valid(41.0, "°C", "Windows"))];
        merge(&mut values, "Temperature", reading(Err(3), "°C"));
        assert_eq!(values[0].1.value, Some(41.0));
        merge(&mut values, "Temperature", reading(Ok(0.0), "°C"));
        assert_eq!(values[0].1.value, Some(0.0));
        assert_eq!(reading(Err(4), "W").state, Availability::PermissionRequired);
        let unavailable_memory = memory_usage(Err(4));
        assert_eq!(unavailable_memory.state, Availability::PermissionRequired);
        assert!(unavailable_memory.detail.contains("NVML status 4"));
        assert_eq!(unavailable_memory.value, None);
        let empty_memory = memory_usage(Ok(Memory {
            total: 4096,
            free: 4096,
            used: 0,
        }));
        assert_eq!(empty_memory.value, Some(0.0));
        assert!(empty_memory.detail.contains("driver-reserved"));
    }
}
