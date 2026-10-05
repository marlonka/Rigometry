use super::{nvml::Nvml, smbios};
use crate::model::{Adapter, Availability, Field, GpuSample, Inventory, Reading, bytes};
use ::windows::{
    Wdk::{
        Graphics::Direct3D::*,
        System::SystemInformation::{
            NtQuerySystemInformation, SystemProcessorPerformanceInformation,
        },
    },
    Win32::{
        Foundation::LUID,
        Graphics::Dxgi::*,
        Storage::FileSystem::GetDiskFreeSpaceExW,
        System::{
            Performance::*, Power::*, SystemInformation::*,
            WindowsProgramming::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
        },
    },
    core::{Interface, PCWSTR},
};
use std::collections::BTreeMap;
use std::{os::windows::ffi::OsStrExt, path::Path};

fn wide(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())])
        .trim()
        .into()
}
fn luid_id(value: LUID) -> String {
    format!("{:08X}:{:08X}", value.HighPart as u32, value.LowPart)
}
fn parse_luid(value: &str) -> Option<LUID> {
    let (hi, lo) = value.split_once(':')?;
    Some(LUID {
        HighPart: u32::from_str_radix(hi, 16).ok()? as i32,
        LowPart: u32::from_str_radix(lo, 16).ok()?,
    })
}

fn pci_field(address: D3DKMT_ADAPTERADDRESS) -> Field {
    if address.BusNumber > 255 || address.DeviceNumber > 31 || address.FunctionNumber > 7 {
        Field::missing(
            "PCI address",
            Availability::Unsupported,
            "D3DKMT_ADAPTERADDRESS",
            format!(
                "No valid PCI function: bus={}, device={}, function={}; software/virtual adapters need not have a physical PCI address",
                address.BusNumber, address.DeviceNumber, address.FunctionNumber
            ),
        )
    } else {
        Field::valid(
            "PCI address",
            format!(
                "{:02X}:{:02X}.{:X}",
                address.BusNumber, address.DeviceNumber, address.FunctionNumber
            ),
            "",
            "D3DKMT_ADAPTERADDRESS; PCI domain not exposed",
        )
    }
}
fn d3d_clock(raw: u64, source: &str) -> Reading {
    // Hardware validation found drivers returning 600/3000 for fields documented
    // in Hz. Never guess a driver-specific multiplier. Zero remains valid.
    if raw > 0 && raw < 1_000_000 {
        let mut reading = Reading::missing(
            "MHz",
            source,
            &format!(
                "Driver returned raw frequency {raw} for a documented Hz field. Nonzero clock below 1 MHz has unverified units; no unit correction is assumed."
            ),
        );
        reading.state = Availability::Failed;
        reading
    } else {
        Reading::valid(raw as f64 / 1_000_000.0, "MHz", source)
    }
}

const CPU_TIMES_SOURCE: &str = "NtQuerySystemInformation / processor time deltas";
pub(super) struct CpuMonitor {
    count: usize,
    previous: Option<Vec<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>>,
}
impl CpuMonitor {
    pub(super) fn new(count: usize) -> Self {
        Self {
            count,
            previous: cpu_times(count).ok(),
        }
    }
    pub(super) fn sample(&mut self) -> (Reading, Vec<Reading>) {
        let missing = |detail: &str| {
            let r = Reading::missing("%", CPU_TIMES_SOURCE, detail);
            (r.clone(), vec![r; self.count])
        };
        let current = match cpu_times(self.count) {
            Ok(v) => v,
            Err(e) => {
                self.previous = None;
                return missing(&e);
            }
        };
        let Some(previous) = self.previous.replace(current.clone()) else {
            return missing("Waiting for two successful processor-time samples");
        };
        if previous.len() != current.len() {
            return missing("Processor topology changed; collecting a new baseline");
        }
        // Up to 4096 independently valid 64-bit deltas can exceed u64.
        let mut busy = 0u128;
        let mut total = 0u128;
        let mut complete = true;
        let readings = previous
            .iter()
            .zip(&current)
            .map(|(old, new)| match cpu_delta(old, new) {
                Ok((b, t)) => {
                    busy += b as u128;
                    total += t as u128;
                    Reading::valid(b as f64 * 100.0 / t as f64, "%", CPU_TIMES_SOURCE)
                }
                Err(e) => {
                    complete = false;
                    let mut r = Reading::missing("%", CPU_TIMES_SOURCE, &e);
                    r.state = Availability::Failed;
                    r
                }
            })
            .collect();
        let overall = if complete && total > 0 {
            Reading::valid(busy as f64 * 100.0 / total as f64, "%", CPU_TIMES_SOURCE)
        } else {
            Reading::missing(
                "%",
                CPU_TIMES_SOURCE,
                "At least one processor time delta was invalid; aggregate not estimated",
            )
        };
        (overall, readings)
    }
}
fn cpu_times(count: usize) -> Result<Vec<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>, String> {
    if count == 0 || count > 4096 {
        return Err("Logical processor count unavailable or outside supported bound".into());
    }
    let mut values = vec![SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION::default(); count];
    let mut returned = 0;
    let status = unsafe {
        NtQuerySystemInformation(
            SystemProcessorPerformanceInformation,
            values.as_mut_ptr().cast(),
            std::mem::size_of_val(values.as_slice()) as u32,
            &mut returned,
        )
    };
    if status.0 < 0 {
        return Err(format!(
            "Processor performance query NTSTATUS 0x{:08X}",
            status.0 as u32
        ));
    }
    if returned as usize != std::mem::size_of_val(values.as_slice()) {
        return Err(format!(
            "Processor performance query returned {returned} bytes for {count} logical processors; incomplete topology"
        ));
    }
    Ok(values)
}
fn cpu_delta(
    old: &SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
    new: &SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
) -> Result<(u64, u64), String> {
    let difference = |a: i64, b: i64| {
        b.checked_sub(a)
            .filter(|n| *n >= 0)
            .map(|n| n as u64)
            .ok_or("Processor time counter moved backwards")
    };
    let idle = difference(old.IdleTime, new.IdleTime)?;
    let kernel = difference(old.KernelTime, new.KernelTime)?;
    let user = difference(old.UserTime, new.UserTime)?;
    let total = kernel
        .checked_add(user)
        .ok_or("Processor time counter overflow")?;
    if total == 0 {
        return Err("Processor time has not advanced; awaiting next sample".into());
    }
    let busy = total
        .checked_sub(idle)
        .ok_or("Idle time exceeds kernel + user time")?;
    Ok((busy, total))
}
pub(super) fn memory() -> (Reading, Reading) {
    let mut info = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    let result = unsafe { GlobalMemoryStatusEx(&mut info) }
        .map_err(|e| e.to_string())
        .and_then(|()| {
            info.ullTotalPhys
                .checked_sub(info.ullAvailPhys)
                .ok_or_else(|| "Available physical memory exceeds total".into())
        });
    match result {
        Ok(used) => (
            Reading::valid(used as f64, "bytes", "GlobalMemoryStatusEx"),
            Reading::valid(info.ullTotalPhys as f64, "bytes", "GlobalMemoryStatusEx"),
        ),
        Err(e) => {
            let r = Reading::missing("bytes", "GlobalMemoryStatusEx", &e);
            (r.clone(), r)
        }
    }
}

pub(super) fn drive_capacity(path: &Path) -> Result<(u64, u64), String> {
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.is_empty() || name.contains(&0) {
        return Err("Drive path is empty or contains a NUL character".into());
    }
    name.push(0);
    let mut total = 0;
    let mut free = 0;
    // Both outputs use the caller's quota scope. Mixing caller-total with
    // volume-free can report free > total on quota-controlled volumes.
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(name.as_ptr()),
            Some(&mut free),
            Some(&mut total),
            None,
        )
    }
    .map_err(|error| format!("GetDiskFreeSpaceEx: {error}"))?;
    checked_drive_capacity(total, free)
}

fn checked_drive_capacity(total: u64, free: u64) -> Result<(u64, u64), String> {
    if free > total {
        Err("Available drive capacity exceeds total capacity".into())
    } else {
        Ok((total, free))
    }
}

pub(super) fn cpu_frequency(count: usize) -> Reading {
    let source = "CallNtPowerInformation / CurrentMhz";
    if count == 0 || count > 4096 {
        return Reading::missing(
            "MHz",
            source,
            "Logical processor count unavailable or outside supported bound",
        );
    }
    let mut info = vec![PROCESSOR_POWER_INFORMATION::default(); count];
    let status = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(info.as_mut_ptr().cast()),
            std::mem::size_of_val(info.as_slice()) as u32,
        )
    };
    if status.0 < 0 {
        return Reading::missing(
            "MHz",
            source,
            &format!(
                "Windows processor frequency query NTSTATUS 0x{:08X}",
                status.0 as u32
            ),
        );
    }
    let mut reading = Reading::valid(
        info.iter().map(|p| p.CurrentMhz as u64).sum::<u64>() as f64 / count as f64,
        "MHz",
        source,
    );
    reading.detail="Mean Windows-reported current frequency across logical processors; not an effective-clock counter measurement.".into();
    reading
}

pub(super) fn discover(result: &mut Inventory) {
    unsafe {
        let signature = FIRMWARE_TABLE_PROVIDER(u32::from_be_bytes(*b"RSMB"));
        let size = GetSystemFirmwareTable(signature, 0, None);
        if size > 0 && size <= 32 * 1024 * 1024 {
            let mut raw = vec![0u8; size as usize];
            let read = GetSystemFirmwareTable(signature, 0, Some(&mut raw));
            if read > 0 && read <= size {
                raw.truncate(read as usize);
                if let Err(error) = smbios::parse(&raw, result) {
                    result.diagnostics.push(format!("SMBIOS parse: {error}"));
                }
            } else {
                result
                    .diagnostics
                    .push("GetSystemFirmwareTable returned no complete SMBIOS table".into());
            }
        } else {
            result.diagnostics.push(format!(
                "SMBIOS unavailable: GetSystemFirmwareTable returned {size} bytes"
            ));
        }
    }
    if result.motherboard.is_empty() {
        result.motherboard.push(Field::missing(
            "Motherboard / BIOS",
            Availability::Unavailable,
            "GetSystemFirmwareTable",
            "Windows did not return readable SMBIOS identification",
        ));
    }
    topology(&mut result.cpu, &mut result.diagnostics);
    match adapters() {
        Ok(adapters) => {
            for (native, mut adapter) in adapters {
                if let Some(kmt) = parse_luid(&adapter.id).and_then(|id| Kmt::open(id).ok()) {
                    match kmt.query::<D3DKMT_ADAPTERADDRESS>() {
                        Ok(address) => adapter.fields.push(pci_field(address)),
                        Err(e) => adapter.fields.push(Field::missing(
                            "PCI address",
                            Availability::Unavailable,
                            "D3DKMT_ADAPTERADDRESS",
                            e,
                        )),
                    }
                    match kmt.query::<D3DKMT_GPUVERSION>() {
                        Ok(version) => {
                            for (label, value) in [
                                ("Architecture", wide(&version.GpuArchitecture)),
                                ("Video BIOS", wide(&version.BiosVersion)),
                            ] {
                                if !value.is_empty() {
                                    adapter.fields.push(Field::valid(
                                        label,
                                        value,
                                        "",
                                        "D3DKMT_GPUVERSION / display driver",
                                    ));
                                } else {
                                    adapter.fields.push(Field::missing(
                                        label,
                                        Availability::Unavailable,
                                        "D3DKMT_GPUVERSION",
                                        "Display driver returned no value",
                                    ));
                                }
                            }
                        }
                        Err(e) => adapter.fields.push(Field::missing(
                            "Architecture / video BIOS",
                            Availability::Unsupported,
                            "D3DKMT_GPUVERSION",
                            e,
                        )),
                    }
                }
                let _ = native;
                result.adapters.push(adapter);
            }
        }
        Err(error) => result
            .diagnostics
            .push(format!("DXGI adapter discovery failed: {error}")),
    }
    match Nvml::load() {
        Ok(nvml) => {
            for adapter in &mut result.adapters {
                if adapter.vendor_id == 0x10de {
                    nvml.describe(adapter);
                }
            }
        }
        Err(error) => {
            if result.adapters.iter().any(|a| a.vendor_id == 0x10de) {
                result.diagnostics.push(format!("NVML: {error}"));
            }
        }
    }
    for adapter in &mut result.adapters {
        if adapter.vendor_id != 0x10de {
            for label in ["Maximum PCIe generation", "Maximum PCIe width"] {
                adapter.fields.push(Field::missing(label,Availability::Unsupported,"Windows / vendor PCIe provider","D3DKMT exposes PCI address and bandwidth, not negotiated link generation/width. No validated vendor PCIe query is enabled for this adapter."));
            }
        }
    }
    if result.adapters.iter().any(|a| a.vendor_id == 0x1002) {
        result.diagnostics.push("AMD: Windows WDDM counters and D3DKMT are enabled. ADLX supports additional sensors, but no ADLX runtime or bindings are bundled; verify AMD SDK redistribution terms and physical AMD hardware before adding it. Unsupported driver queries stay unavailable.".into());
    }
    if result.adapters.iter().any(|a| a.vendor_id == 0x8086) {
        result.diagnostics.push("Intel: Windows WDDM counters and D3DKMT are enabled. IGCL offers additional driver-specific telemetry; no IGCL binding is bundled. IGCL device mapping and sensor support require physical Intel GPU verification.".into());
    }
    result.diagnostics.push("GPU identity: DXGI LUID binds Windows counters and D3DKMT to the same adapter. NVIDIA sensors require a unique NVML PCI bus/device/function + vendor/device match. No provider is matched by enumeration order.".into());
    result.diagnostics.push("DXGI CurrentUsage and Budget are process-scoped; labels explicitly identify this process. WDDM GPU Adapter Memory counters report adapter usage. NVML used memory includes driver-reserved allocation.".into());
}

fn adapters() -> Result<Vec<(IDXGIAdapter1, Adapter)>, String> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for index in 0..64 {
            let native = match factory.EnumAdapters1(index) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e.to_string()),
            };
            let desc = native.GetDesc1().map_err(|e| e.to_string())?;
            let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
            let id = luid_id(desc.AdapterLuid);
            let mut fields = vec![
                Field::valid(
                    "Vendor",
                    match desc.VendorId {
                        0x10de => "NVIDIA",
                        0x1002 => "AMD",
                        0x8086 => "Intel",
                        0x1414 => "Microsoft",
                        _ => "Other",
                    },
                    "",
                    "DXGI vendor ID",
                ),
                Field::valid(
                    "Vendor / device ID",
                    format!("{:04X}:{:04X}", desc.VendorId, desc.DeviceId),
                    "",
                    "DXGI",
                ),
                Field::valid(
                    "Subsystem / revision",
                    format!("{:08X} / {:02X}", desc.SubSysId, desc.Revision),
                    "",
                    "DXGI",
                ),
                Field::valid(
                    "Adapter type",
                    if software {
                        "Software renderer"
                    } else {
                        "Hardware adapter"
                    },
                    "",
                    "DXGI flags",
                ),
                Field::valid(
                    "Dedicated VRAM capacity",
                    bytes(desc.DedicatedVideoMemory as u64),
                    "",
                    "DXGI; capacity, not usage",
                ),
                Field::valid(
                    "Dedicated system memory",
                    bytes(desc.DedicatedSystemMemory as u64),
                    "",
                    "DXGI; capacity, not usage",
                ),
                Field::valid(
                    "Shared system memory limit",
                    bytes(desc.SharedSystemMemory as u64),
                    "",
                    "DXGI; maximum share, not reserved or used",
                ),
                Field::valid("LUID", &id, "", "DXGI; stable for the Windows boot session"),
            ];
            match native.CheckInterfaceSupport(&IDXGIDevice::IID) {
                Ok(version) => {
                    let v = version as u64;
                    fields.push(Field::valid(
                        "Driver version",
                        format!(
                            "{}.{}.{}.{}",
                            v >> 48,
                            (v >> 32) & 0xffff,
                            (v >> 16) & 0xffff,
                            v & 0xffff
                        ),
                        "",
                        "DXGI CheckInterfaceSupport / IDXGIDevice",
                    ));
                }
                Err(error) => fields.push(Field::missing(
                    "Driver version",
                    Availability::Unavailable,
                    "DXGI CheckInterfaceSupport",
                    error.to_string(),
                )),
            }
            result.push((
                native,
                Adapter {
                    id,
                    name: wide(&desc.Description),
                    vendor_id: desc.VendorId,
                    device_id: desc.DeviceId,
                    fields,
                },
            ));
        }
        Ok(result)
    }
}

fn topology(fields: &mut Vec<Field>, diagnostics: &mut Vec<String>) {
    unsafe {
        let mut len = 0;
        let _ = GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len);
        if len == 0 || len > 16 * 1024 * 1024 {
            return;
        }
        let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
        if let Err(error) = GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            Some(buffer.as_mut_ptr().cast()),
            &mut len,
        ) {
            diagnostics.push(format!("CPU topology: {error}"));
            return;
        }
        let classes = match native_bytes(&buffer, len as usize).and_then(topology_classes) {
            Ok(classes) => classes,
            Err(error) => {
                diagnostics.push(format!("CPU topology: {error}"));
                return;
            }
        };
        if !classes.is_empty() {
            fields.push(Field::valid(
                "Core efficiency classes",
                classes
                    .iter()
                    .map(|(class, n)| format!("Class {class}: {n} cores"))
                    .collect::<Vec<_>>()
                    .join(" · "),
                "",
                "GetLogicalProcessorInformationEx; higher class = higher relative performance",
            ));
        }
    }
}

fn native_bytes(buffer: &[u64], returned: usize) -> Result<&[u8], String> {
    if returned > std::mem::size_of_val(buffer) {
        return Err("Native provider returned a length exceeding its buffer".into());
    }
    // The entire allocation is initialized and returned was checked before
    // constructing this byte view. No provider-supplied pointer is followed.
    Ok(unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast(), returned) })
}

fn topology_classes(data: &[u8]) -> Result<BTreeMap<u8, usize>, String> {
    let mut classes = BTreeMap::<u8, usize>::new();
    let mut pos = 0;
    while pos < data.len() {
        let tail = &data[pos..];
        if tail.len() < 32 {
            return Err("Truncated processor topology record".into());
        }
        let relation = u32::from_le_bytes(tail[..4].try_into().unwrap());
        let size = u32::from_le_bytes(tail[4..8].try_into().unwrap()) as usize;
        let groups = u16::from_le_bytes(tail[30..32].try_into().unwrap()) as usize;
        // Header + PROCESSOR_RELATIONSHIP prefix + variable GROUP_AFFINITYs.
        let minimum = 32 + groups * std::mem::size_of::<GROUP_AFFINITY>();
        if relation != RelationProcessorCore.0 as u32
            || groups == 0
            || size < minimum
            || size > tail.len()
        {
            return Err("Malformed processor topology record".into());
        }
        *classes.entry(tail[9]).or_default() += 1;
        pos += size;
    }
    Ok(classes)
}

// Keep the driver query's structure and selector inseparable. These are the
// only accepted output types; each is a generated C struct with integer fields.
trait KmtOutput: Default {
    const KIND: KMTQUERYADAPTERINFOTYPE;
}
impl KmtOutput for D3DKMT_ADAPTERADDRESS {
    const KIND: KMTQUERYADAPTERINFOTYPE = KMTQAITYPE_ADAPTERADDRESS;
}
impl KmtOutput for D3DKMT_GPUVERSION {
    const KIND: KMTQUERYADAPTERINFOTYPE = KMTQUITYPE_GPUVERSION;
}
impl KmtOutput for D3DKMT_ADAPTER_PERFDATA {
    const KIND: KMTQUERYADAPTERINFOTYPE = KMTQAITYPE_ADAPTERPERFDATA;
}
impl KmtOutput for D3DKMT_NODE_PERFDATA {
    const KIND: KMTQUERYADAPTERINFOTYPE = KMTQAITYPE_NODEPERFDATA;
}

struct Kmt(u32);
impl Kmt {
    fn open(luid: LUID) -> Result<Self, String> {
        unsafe {
            let mut args = D3DKMT_OPENADAPTERFROMLUID {
                AdapterLuid: luid,
                ..Default::default()
            };
            let status = D3DKMTOpenAdapterFromLuid(&mut args);
            if status.0 < 0 {
                Err(format!(
                    "D3DKMTOpenAdapterFromLuid NTSTATUS 0x{:08X}",
                    status.0 as u32
                ))
            } else {
                Ok(Self(args.hAdapter))
            }
        }
    }
    fn query<T: KmtOutput>(&self) -> Result<T, String> {
        unsafe {
            let mut value = T::default();
            let mut args = D3DKMT_QUERYADAPTERINFO {
                hAdapter: self.0,
                Type: T::KIND,
                pPrivateDriverData: (&mut value as *mut T).cast(),
                PrivateDriverDataSize: std::mem::size_of::<T>() as u32,
            };
            let status = D3DKMTQueryAdapterInfo(&mut args);
            if status.0 < 0 {
                Err(format!(
                    "Display driver query {} returned NTSTATUS 0x{:08X}",
                    T::KIND.0,
                    status.0 as u32
                ))
            } else {
                Ok(value)
            }
        }
    }
}
impl Drop for Kmt {
    fn drop(&mut self) {
        unsafe {
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: self.0 });
        }
    }
}

struct LiveAdapter {
    adapter: Adapter,
    dxgi: Option<IDXGIAdapter3>,
    kmt: Option<Kmt>,
}
pub(super) struct GpuMonitor {
    adapters: Vec<LiveAdapter>,
    pdh: Result<Pdh, String>,
    nvml: Result<Nvml, String>,
}
impl GpuMonitor {
    pub(super) fn new(inventory: &[Adapter]) -> Self {
        let native = adapters().unwrap_or_default();
        let adapters = inventory
            .iter()
            .map(|adapter| LiveAdapter {
                adapter: adapter.clone(),
                dxgi: native
                    .iter()
                    .find(|(_, a)| a.id == adapter.id)
                    .and_then(|(n, _)| n.cast().ok()),
                kmt: parse_luid(&adapter.id).and_then(|luid| Kmt::open(luid).ok()),
            })
            .collect();
        Self {
            adapters,
            pdh: Pdh::new(),
            nvml: Nvml::load(),
        }
    }
    pub(super) fn sample(&mut self) -> Vec<GpuSample> {
        let counters = match &self.pdh {
            Ok(pdh) => pdh.sample(),
            Err(e) => Err(e.clone()),
        };
        self.adapters.iter().map(|live| {
            let mut readings=Vec::new();
            let id=&live.adapter.id;
            for (label,unit,key) in [("Utilization","%",0),("Dedicated memory used","bytes",1),("Shared memory used","bytes",2)] {
                let reading=match &counters {
                    Ok(values)=>values.reading(id,key,unit),
                    Err(e)=>Reading::missing(unit,"Windows WDDM performance counters",e),
                };
                readings.push((label.into(),reading));
            }
            for (segment,prefix) in [(DXGI_MEMORY_SEGMENT_GROUP_LOCAL,"Process local"),(DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,"Process non-local")] {
                let mut info=DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                let query=live.dxgi.as_ref().ok_or_else(||"IDXGIAdapter3 is unavailable".into()).and_then(|dxgi|unsafe {dxgi.QueryVideoMemoryInfo(0,segment,&mut info)}.map_err(|e|e.to_string()));
                for (label,value) in [("usage",info.CurrentUsage),("budget",info.Budget)] {
                    readings.push((format!("{prefix} {label}"),match &query {Ok(())=>Reading::valid(value as f64,"bytes","DXGI QueryVideoMemoryInfo; this process only"),Err(e)=>Reading::missing("bytes","DXGI QueryVideoMemoryInfo; this process only",e)}));
                }
            }
            let perf=live.kmt.as_ref().ok_or_else(||"D3DKMT adapter handle unavailable".into()).and_then(|k|k.query::<D3DKMT_ADAPTER_PERFDATA>());
            for (label,unit,value) in [("Temperature","°C",perf.as_ref().map(|p|p.Temperature as f64 / 10.0)),("Fan","RPM",perf.as_ref().map(|p|p.FanRPM as f64)),("Power (driver percentage)","%",perf.as_ref().map(|p|p.Power as f64 / 10.0))] {
                readings.push((label.into(),match value {Ok(v)=>Reading::valid(v,unit,"D3DKMT_ADAPTER_PERFDATA / WDDM driver"),Err(e)=>unsupported(unit,"D3DKMT_ADAPTER_PERFDATA",e)}));
            }
            readings.push(("Memory clock".into(),match &perf {Ok(p)=>d3d_clock(p.MemoryFrequency,"D3DKMT_ADAPTER_PERFDATA / WDDM driver"),Err(e)=>unsupported("MHz","D3DKMT_ADAPTER_PERFDATA",e)}));
            let node=live.kmt.as_ref().ok_or_else(||"D3DKMT adapter handle unavailable".into()).and_then(|k|k.query::<D3DKMT_NODE_PERFDATA>());
            readings.push(("Engine 0 clock".into(),match &node {Ok(p)=>d3d_clock(p.Frequency,"D3DKMT_NODE_PERFDATA / engine 0"),Err(e)=>unsupported("MHz","D3DKMT_NODE_PERFDATA",e)}));
            readings.push(("Engine 0 voltage".into(),match &node {Ok(p)=>Reading::valid(p.Voltage as f64,"mV","D3DKMT_NODE_PERFDATA / engine 0"),Err(e)=>unsupported("mV","D3DKMT_NODE_PERFDATA",e)}));
            if live.adapter.vendor_id==0x10de {
                match &self.nvml {
                    Ok(nvml)=>nvml.sample(&live.adapter,&mut readings),
                    Err(e)=>readings.push(("NVIDIA provider".into(),Reading::missing("","NVML",e))),
                }
            } else {
                for (label,unit) in [("PCIe generation",""),("PCIe width","lanes")] {
                    let mut reading=Reading::missing(unit,"Windows / vendor PCIe provider","Current link generation/width is not exposed by the enabled provider for this adapter.");reading.state=Availability::Unsupported;
                    readings.push((label.into(),reading));
                }
            }
            GpuSample {adapter_id:id.clone(),readings}
        }).collect()
    }
}
fn unsupported(unit: &str, source: &str, detail: &str) -> Reading {
    let mut r = Reading::missing(unit, source, detail);
    r.state = if detail.contains("0xC0000022") {
        Availability::PermissionRequired
    } else if detail.contains("0xC0000002") || detail.contains("0xC00000BB") {
        Availability::Unsupported
    } else if detail.contains("unavailable") {
        Availability::Unavailable
    } else {
        Availability::Failed
    };
    r
}

struct Pdh {
    query: PDH_HQUERY,
    counters: Vec<(u8, PDH_HCOUNTER)>,
}

#[derive(Default)]
struct CounterSnapshot {
    values: BTreeMap<(String, u8), f64>,
    errors: BTreeMap<u8, String>,
}
impl CounterSnapshot {
    fn reading(&self, adapter: &str, kind: u8, unit: &str) -> Reading {
        let source = "Windows WDDM performance counters";
        if let Some(error) = self.errors.get(&kind) {
            return Reading::missing(unit, source, error);
        }
        self.values
            .get(&(adapter.to_owned(), kind))
            .map(|value| {
                let mut reading = Reading::valid(*value, unit, source);
                if kind == 0 {
                    reading.detail = "Busiest hardware engine after summing process instances per engine; approximately one-second sample interval.".into();
                }
                reading
            })
            .unwrap_or_else(|| Reading::missing(unit, source, "No valid counter instance for this adapter; requires a supporting WDDM driver"))
    }
}

fn collect_counters(
    counters: impl IntoIterator<Item = (u8, Result<Vec<(String, f64)>, String>)>,
) -> CounterSnapshot {
    let mut snapshot = CounterSnapshot::default();
    let mut engines = BTreeMap::<(String, String), f64>::new();
    for (kind, entries) in counters {
        let entries = match entries {
            Ok(entries) => entries,
            Err(error) => {
                snapshot.errors.insert(kind, error);
                continue;
            }
        };
        for (name, value) in entries {
            if !value.is_finite() || value < 0.0 {
                continue;
            }
            if let Some((id, engine)) = counter_identity(&name) {
                if kind == 0 {
                    *engines.entry((id, engine)).or_default() += value;
                } else {
                    *snapshot.values.entry((id, kind)).or_insert(0.0) += value;
                }
            }
        }
    }
    for ((id, _), value) in engines {
        let entry = snapshot.values.entry((id, 0)).or_insert(0.0);
        *entry = f64::max(*entry, value.clamp(0.0, 100.0));
    }
    snapshot
}

impl Pdh {
    fn new() -> Result<Self, String> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                return Err(format!("PdhOpenQueryW 0x{status:08X}"));
            }
            let mut result = Self {
                query,
                counters: Vec::new(),
            };
            for (kind, path) in [
                (0, r"\GPU Engine(*)\Utilization Percentage"),
                (1, r"\GPU Adapter Memory(*)\Dedicated Usage"),
                (2, r"\GPU Adapter Memory(*)\Shared Usage"),
            ] {
                let path = path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
                let mut counter = PDH_HCOUNTER::default();
                if PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) == 0 {
                    result.counters.push((kind, counter));
                }
            }
            if result.counters.is_empty() {
                return Err("GPU performance counter objects are unavailable; requires a supporting WDDM driver".into());
            }
            let _ = PdhCollectQueryData(query);
            Ok(result)
        }
    }
    fn sample(&self) -> Result<CounterSnapshot, String> {
        unsafe {
            let status = PdhCollectQueryData(self.query);
            if status != 0 {
                return Err(format!("PdhCollectQueryData 0x{status:08X}"));
            }
            Ok(collect_counters(
                self.counters
                    .iter()
                    .map(|(kind, counter)| (*kind, counter_values(*counter))),
            ))
        }
    }
}
impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

unsafe fn counter_values(counter: PDH_HCOUNTER) -> Result<Vec<(String, f64)>, String> {
    unsafe {
        for _ in 0..3 {
            // PDH explicitly says a failed nonzero-sized read does not provide
            // a reliable new size. Probe again after every instance-count race.
            let mut size = 0;
            let mut count = 0;
            let status =
                PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if status == PDH_CSTATUS_NO_INSTANCE {
                return Ok(Vec::new());
            }
            if status != PDH_MORE_DATA && status != 0 {
                return Err(format!("GPU counter read 0x{status:08X}"));
            }
            if size == 0 {
                return Ok(Vec::new());
            }
            if size > 64 * 1024 * 1024 {
                return Err("GPU counter buffer exceeds safety bound".into());
            }
            let mut data = vec![0u64; (size as usize).div_ceil(8)];
            let capacity = data.len() * 8;
            size = capacity as u32;
            let status = PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(data.as_mut_ptr().cast()),
            );
            if status == PDH_MORE_DATA {
                continue;
            }
            if status != 0 {
                return Err(format!("GPU counter array 0x{status:08X}"));
            }
            return parse_counter_buffer(&data, size as usize, count as usize);
        }
        Err("GPU counter instances changed repeatedly during collection".into())
    }
}

fn parse_counter_buffer(
    buffer: &[u64],
    returned: usize,
    count: usize,
) -> Result<Vec<(String, f64)>, String> {
    let data = native_bytes(buffer, returned)?;
    let item_size = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
    if count > 65_536 || count > data.len() / item_size {
        return Err("GPU counter count exceeds buffer or safety bound".into());
    }
    let names_start = count * item_size;
    let mut name_bytes = 0usize;
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        // The header range is checked above. read_unaligned avoids assuming
        // alignment from a provider; this C struct contains only raw pointers,
        // integer status and a numeric union (all bit patterns are valid).
        let entry = unsafe {
            data.as_ptr()
                .add(index * item_size)
                .cast::<PDH_FMT_COUNTERVALUE_ITEM_W>()
                .read_unaligned()
        };
        if entry.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA
            && entry.FmtValue.CStatus != PDH_CSTATUS_NEW_DATA
        {
            continue;
        }
        let start = (entry.szName.0 as usize)
            .checked_sub(data.as_ptr() as usize)
            .filter(|start| *start >= names_start && *start < data.len() && start % 2 == 0)
            .ok_or("GPU counter name points outside its string buffer")?;
        // Bound both the search and total copied strings: repeated/shared
        // provider pointers must not multiply a small buffer into huge output.
        let end = data.len().min(start.saturating_add(8192));
        let tail = &data[start..end];
        let length = tail
            .as_chunks::<2>()
            .0
            .iter()
            .position(|c| *c == [0, 0])
            .ok_or("GPU counter name is unterminated or exceeds 4095 UTF-16 units")?;
        name_bytes += length * 2;
        if name_bytes > 8 * 1024 * 1024 {
            return Err("GPU counter names exceed safety bound".into());
        }
        let name = String::from_utf16_lossy(
            &tail[..length * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        );
        // PDH_FMT_DOUBLE selects this union member at the call boundary.
        result.push((name, unsafe { entry.FmtValue.Anonymous.doubleValue }));
    }
    Ok(result)
}

fn counter_identity(name: &str) -> Option<(String, String)> {
    let start = name.find("luid_")? + 5;
    let mut parts = name[start..].splitn(3, '_');
    let hi = u32::from_str_radix(parts.next()?.strip_prefix("0x")?, 16).ok()?;
    let lo = u32::from_str_radix(parts.next()?.strip_prefix("0x")?, 16).ok()?;
    Some((
        format!("{hi:08X}:{lo:08X}"),
        parts.next().unwrap_or("").to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_gpu_counters_survive_another_counter_failure() {
        let memory = "luid_0x00000000_0x0000abcd_phys_0".to_string();
        let snapshot = collect_counters([
            (0, Err("Engine samples are not ready".into())),
            (1, Ok(vec![(memory.clone(), 4096.0)])),
            (2, Ok(vec![(memory, 0.0)])),
        ]);
        let id = "00000000:0000ABCD";
        assert_eq!(snapshot.reading(id, 1, "bytes").value, Some(4096.0));
        assert_eq!(snapshot.reading(id, 2, "bytes").value, Some(0.0));
        let usage = snapshot.reading(id, 0, "%");
        assert_eq!(usage.state, Availability::Unavailable);
        assert!(usage.detail.contains("not ready"));

        let snapshot = collect_counters([
            (
                0,
                Ok(vec![
                    (
                        "pid_1_luid_0x00000000_0x0000abcd_phys_0_eng_0_engtype_3D".into(),
                        10.0,
                    ),
                    (
                        "pid_2_luid_0x00000000_0x0000abcd_phys_0_eng_0_engtype_3D".into(),
                        20.0,
                    ),
                    (
                        "pid_1_luid_0x00000000_0x0000abcd_phys_0_eng_1_engtype_Copy".into(),
                        25.0,
                    ),
                ]),
            ),
            (1, Err("Memory counter disappeared".into())),
        ]);
        assert_eq!(snapshot.reading(id, 0, "%").value, Some(30.0));
        assert_eq!(snapshot.reading(id, 1, "bytes").value, None);
        assert_eq!(snapshot.reading("00000000:0000ABCE", 0, "%").value, None);
    }

    #[test]
    fn unavailable_drive_capacity_is_not_a_valid_zero_but_full_disks_are() {
        let directory = tempfile::tempdir().unwrap();
        let (total, free) = drive_capacity(directory.path()).unwrap();
        assert!(total >= free);
        assert!(drive_capacity(&directory.path().join("nonexistent-mount")).is_err());
        assert_eq!(checked_drive_capacity(4096, 0).unwrap(), (4096, 0));
        assert_eq!(checked_drive_capacity(0, 0).unwrap(), (0, 0));
        assert!(checked_drive_capacity(4096, 4097).is_err());
    }

    #[test]
    fn native_counter_names_and_lengths_cannot_escape_the_returned_buffer() {
        let item_size = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let mut words = vec![0u64; (item_size + 16).div_ceil(8)];
        let capacity = std::mem::size_of_val(words.as_slice());
        let base = words.as_ptr() as usize;
        let write_entry = |words: &mut [u64], pointer: usize| {
            let entry = PDH_FMT_COUNTERVALUE_ITEM_W {
                szName: ::windows::core::PWSTR(pointer as *mut u16),
                FmtValue: PDH_FMT_COUNTERVALUE {
                    CStatus: PDH_CSTATUS_VALID_DATA,
                    Anonymous: PDH_FMT_COUNTERVALUE_0 { doubleValue: 0.0 },
                },
            };
            // Test fixture mirrors the documented caller-owned PDH buffer.
            unsafe {
                words
                    .as_mut_ptr()
                    .cast::<PDH_FMT_COUNTERVALUE_ITEM_W>()
                    .write(entry)
            };
        };
        write_entry(&mut words, base + item_size);
        assert_eq!(
            parse_counter_buffer(&words, capacity, 1).unwrap(),
            vec![(String::new(), 0.0)]
        );
        for pointer in [0, base, base + item_size + 1, base + capacity] {
            write_entry(&mut words, pointer);
            assert!(parse_counter_buffer(&words, capacity, 1).is_err());
        }
        write_entry(&mut words, base + item_size);
        assert!(parse_counter_buffer(&words, capacity + 1, 1).is_err());
        assert!(parse_counter_buffer(&words, item_size - 1, 1).is_err());
        assert!(parse_counter_buffer(&words, capacity, usize::MAX).is_err());
        for word in &mut words[item_size / 8..] {
            *word = u64::MAX;
        }
        assert!(parse_counter_buffer(&words, capacity, 1).is_err());
    }

    #[test]
    fn topology_rejects_truncated_records_and_impossible_group_lengths() {
        let size = 32 + std::mem::size_of::<GROUP_AFFINITY>();
        let mut data = vec![0u8; size];
        data[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        data[9] = 7;
        data[30..32].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(topology_classes(&data).unwrap().get(&7), Some(&1));
        assert!(native_bytes(&[0u64; 2], 17).is_err());
        assert!(topology_classes(&data[..size - 1]).is_err());
        data[30..32].copy_from_slice(&2u16.to_le_bytes());
        assert!(topology_classes(&data).is_err());
        data[30..32].copy_from_slice(&1u16.to_le_bytes());
        data.push(0);
        assert!(
            topology_classes(&data).is_err(),
            "Trailing fragment cannot produce partial valid topology"
        );
    }
    #[test]
    fn cpu_idle_zero_is_valid_but_no_elapsed_time_and_reversed_counters_are_not() {
        let baseline = SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION::default();
        let idle = SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION {
            IdleTime: 100,
            KernelTime: 100,
            ..Default::default()
        };
        assert_eq!(cpu_delta(&baseline, &idle).unwrap(), (0, 100));
        let active = SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION {
            IdleTime: 75,
            KernelTime: 90,
            UserTime: 10,
            ..Default::default()
        };
        assert_eq!(cpu_delta(&baseline, &active).unwrap(), (25, 100));
        assert!(cpu_delta(&baseline, &baseline).is_err());
        assert!(cpu_delta(&active, &baseline).is_err());
        let inconsistent = SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION {
            IdleTime: 101,
            KernelTime: 100,
            ..Default::default()
        };
        assert!(cpu_delta(&baseline, &inconsistent).is_err());
    }
    #[test]
    fn unverified_driver_frequency_units_are_rejected_without_reinterpreting_zero() {
        assert_eq!(d3d_clock(3_000, "driver").state, Availability::Failed);
        assert_eq!(d3d_clock(600, "driver").value, None);
        assert_eq!(d3d_clock(0, "driver").value, Some(0.0));
        assert_eq!(d3d_clock(1_500_000_000, "driver").value, Some(1500.0));
    }
    #[test]
    fn software_pci_sentinel_is_not_a_physical_identity() {
        let invalid = pci_field(D3DKMT_ADAPTERADDRESS {
            BusNumber: u32::MAX,
            DeviceNumber: 0xffff,
            FunctionNumber: 0xffff,
        });
        assert_eq!(invalid.state, Availability::Unsupported);
        assert!(invalid.value.is_none());
        let valid = pci_field(D3DKMT_ADAPTERADDRESS::default());
        assert_eq!(valid.value.as_deref(), Some("00:00.0"));
    }
    #[test]
    fn adapter_identity_never_depends_on_provider_order() {
        let a =
            counter_identity("pid_980_luid_0x00000000_0x0000abcd_phys_0_eng_2_engtype_3D").unwrap();
        let b =
            counter_identity("pid_12_luid_0x00000000_0x0000abcd_phys_0_eng_2_engtype_3D").unwrap();
        let other =
            counter_identity("pid_12_luid_0x00000001_0x0000abcd_phys_0_eng_2_engtype_3D").unwrap();
        assert_eq!(a, b);
        assert_ne!(a.0, other.0);
        assert_eq!(a.0, "00000000:0000ABCD");
        assert!(counter_identity("luid_0xgarbage_0x123").is_none());
    }
    #[test]
    fn luid_round_trip_preserves_signed_high_bits() {
        let luid = parse_luid("FFFF8000:ABCDEF12").unwrap();
        assert_eq!(luid_id(luid), "FFFF8000:ABCDEF12");
    }
}
