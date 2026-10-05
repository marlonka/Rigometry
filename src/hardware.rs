//! Read-only hardware discovery and telemetry. Construct and poll on the sensor worker.
#[cfg(windows)]
mod nvml;
mod smbios;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use crate::model::Reading;
use crate::model::{Availability, Drive, Field, Inventory, Telemetry, bytes, now_ms};
use sysinfo::{Disks, System};

pub fn discover() -> Inventory {
    let mut result = Inventory::default();
    let mut system = System::new();
    system.refresh_cpu_all();
    system.refresh_memory();
    let cpuid = raw_cpuid::CpuId::new();
    if let Some(brand) = cpuid.get_processor_brand_string() {
        result.cpu.push(Field::valid(
            "Model",
            brand.as_str().trim(),
            "",
            "CPUID brand string",
        ));
    }
    if let Some(vendor) = cpuid.get_vendor_info() {
        result
            .cpu
            .push(Field::valid("Vendor", vendor.as_str(), "", "CPUID leaf 0"));
    }
    result.cpu.push(Field::valid(
        "Architecture",
        std::env::consts::ARCH,
        "",
        "Executable target",
    ));
    if let Some(cores) = System::physical_core_count() {
        result.cpu.push(Field::valid(
            "Physical cores",
            cores.to_string(),
            "",
            "Windows processor topology / sysinfo",
        ));
    }
    result.cpu.push(Field::valid(
        "Logical processors",
        system.cpus().len().to_string(),
        "",
        "Windows / sysinfo",
    ));
    cpu_details(&mut result.cpu);
    #[cfg(windows)]
    {
        let usable = windows::memory().1;
        result.memory.push(Field {
            label: "Windows usable memory".into(),
            value: usable.value.map(|v| bytes(v as u64)),
            unit: String::new(),
            source: usable.source,
            state: usable.state,
            detail: usable.detail,
            timestamp_ms: usable.timestamp_ms,
        });
    }
    #[cfg(not(windows))]
    result.memory.push(Field::valid(
        "Windows usable memory",
        bytes(system.total_memory()),
        "",
        "sysinfo",
    ));
    for (label, value) in [
        ("Windows", System::long_os_version()),
        ("Build", System::os_version()),
        ("Host", System::host_name()),
    ] {
        if let Some(value) = value {
            result
                .os
                .push(Field::valid(label, value, "", "Windows / sysinfo"));
        }
    }
    result.os.push(Field::valid(
        "System architecture",
        System::cpu_arch(),
        "",
        "Windows / sysinfo",
    ));
    for disk in Disks::new_with_refreshed_list().list() {
        #[cfg(windows)]
        let (total_bytes, free_bytes) = match windows::drive_capacity(disk.mount_point()) {
            Ok(capacity) => capacity,
            Err(error) => {
                result.diagnostics.push(format!(
                    "Drive {} ({}) omitted: capacity is unavailable, not zero. {error}",
                    disk.mount_point().display(),
                    disk.name().to_string_lossy(),
                ));
                continue;
            }
        };
        #[cfg(not(windows))]
        let (total_bytes, free_bytes) = (disk.total_space(), disk.available_space());
        result.drives.push(Drive {
            mount: disk.mount_point().display().to_string(),
            name: disk.name().to_string_lossy().into_owned(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
            total_bytes,
            free_bytes,
        });
    }
    #[cfg(windows)]
    result.diagnostics.push("Drive capacity/free space: GetDiskFreeSpaceEx values available to this user; per-user quotas can reduce these values. A failed query omits the drive and records its mount above instead of fabricating zero capacity.".into());
    #[cfg(windows)]
    windows::discover(&mut result);
    for (label, source, detail) in [
        (
            "Temperature",
            "CPU digital thermal sensors",
            "Windows exposes no universal unprivileged CPU package sensor. Requires a supported, signed hardware access provider; none is bundled.",
        ),
        (
            "Core voltage",
            "CPU voltage sensors",
            "Requires a model-specific vendor or signed privileged provider; none is bundled.",
        ),
        (
            "Package power",
            "CPU energy counters",
            "RAPL / vendor energy counters require supported model-specific privileged access; none is bundled.",
        ),
    ] {
        result.cpu.push(Field::missing(
            label,
            Availability::Unsupported,
            source,
            detail,
        ));
    }
    result.memory.push(Field::missing("Channel mode", Availability::Unsupported, "Memory controller", "SMBIOS module positions do not establish active channel mode; no memory-controller provider is bundled."));
    result.memory.push(Field::missing("SPD / timings / XMP / EXPO", Availability::Unsupported, "SPD / memory controller", "SMBIOS exposes module identification and transfer rates only. SPD profiles and active timings need supported SMBus / memory-controller access; no signed provider is bundled."));
    result.diagnostics.push("Discovery is read-only. CPUID cache descriptions describe the executing processor's cache topology. Firmware values may be incomplete or incorrect; their SMBIOS source remains visible.".into());
    result
}

#[cfg(target_arch = "x86_64")]
fn cpu_details(fields: &mut Vec<Field>) {
    use std::arch::x86_64::__cpuid_count;
    // CPUID is guaranteed on the x86-64 target. Leaves are guarded by their maximum.
    let read = |leaf, sub| __cpuid_count(leaf, sub);
    let max = read(0, 0).eax;
    let ext = read(0x8000_0000, 0).eax;
    let f = read(1, 0);
    let base_family = (f.eax >> 8) & 15;
    let family = base_family
        + if base_family == 15 {
            (f.eax >> 20) & 255
        } else {
            0
        };
    let model = ((f.eax >> 4) & 15)
        + if base_family == 6 || base_family == 15 {
            ((f.eax >> 16) & 15) << 4
        } else {
            0
        };
    fields.push(Field::valid(
        "Family / model / stepping",
        format!("{family} / {model} / {}", f.eax & 15),
        "",
        "CPUID leaf 1",
    ));
    let mut features = Vec::new();
    for (bit, name) in [(23, "MMX"), (25, "SSE"), (26, "SSE2")] {
        if f.edx & (1 << bit) != 0 {
            features.push(name);
        }
    }
    for (bit, name) in [
        (0, "SSE3"),
        (9, "SSSE3"),
        (12, "FMA"),
        (19, "SSE4.1"),
        (20, "SSE4.2"),
        (25, "AES"),
        (28, "AVX"),
        (29, "F16C"),
        (30, "RDRAND"),
    ] {
        if f.ecx & (1 << bit) != 0 {
            features.push(name);
        }
    }
    if max >= 7 {
        let e = read(7, 0);
        for (bit, name) in [
            (3, "BMI1"),
            (5, "AVX2"),
            (8, "BMI2"),
            (16, "AVX-512F"),
            (18, "RDSEED"),
            (19, "ADX"),
            (29, "SHA"),
        ] {
            if e.ebx & (1 << bit) != 0 {
                features.push(name);
            }
        }
        for (bit, name) in [(8, "GFNI"), (9, "VAES"), (10, "VPCLMULQDQ")] {
            if e.ecx & (1 << bit) != 0 {
                features.push(name);
            }
        }
    }
    if ext >= 0x8000_0001 {
        let e = read(0x8000_0001, 0);
        if e.ecx & (1 << 5) != 0 {
            features.push("LZCNT");
        }
        if e.edx & (1 << 29) != 0 {
            features.push("x86-64");
        }
    }
    fields.push(Field::valid(
        "Instruction sets (hardware)",
        features.join(" · "),
        "",
        "CPUID hardware support; OS enablement not implied",
    ));
    if max >= 0x16 {
        let freq = read(0x16, 0);
        for (label, value) in [
            ("Nominal frequency", freq.eax & 0xffff),
            ("Maximum nominal frequency", freq.ebx & 0xffff),
            ("Bus frequency", freq.ecx & 0xffff),
        ] {
            if value != 0 {
                fields.push(Field::valid(
                    label,
                    value.to_string(),
                    "MHz",
                    "CPUID leaf 16h; nominal, not measured",
                ));
            } else {
                fields.push(Field::missing(label,Availability::Unsupported,"CPUID leaf 16h","The processor returned zero (frequency information unavailable in this CPUID leaf)"));
            }
        }
    } else {
        for label in [
            "Nominal frequency",
            "Maximum nominal frequency",
            "Bus frequency",
        ] {
            fields.push(Field::missing(label,Availability::Unsupported,"CPUID leaf 16h","This processor does not implement CPUID frequency leaf 16h; firmware-configured and Windows-reported frequencies are shown separately"));
        }
    }
    let cache_leaf = if ext >= 0x8000_001d && read(0x8000_0001, 0).ecx & (1 << 22) != 0 {
        Some(0x8000_001d)
    } else if max >= 4 {
        Some(4)
    } else {
        None
    };
    if let Some(leaf) = cache_leaf {
        for sub in 0..32 {
            let c = read(leaf, sub);
            let ty = c.eax & 31;
            if ty == 0 {
                break;
            }
            let Some(size) = cache_size(c.ebx, c.ecx) else {
                fields.push(Field::missing(
                    "Cache capacity",
                    Availability::Failed,
                    format!("CPUID {leaf:08X}h subleaf {sub}"),
                    "Cache geometry exceeds a 64-bit byte count",
                ));
                continue;
            };
            let kind = match ty {
                1 => "data",
                2 => "instruction",
                _ => "unified",
            };
            fields.push(Field::valid(
                format!("L{} {kind} cache", (c.eax >> 5) & 7),
                format!(
                    "{} · {}-way · shared by ≤{} threads",
                    bytes(size),
                    ((c.ebx >> 22) & 1023) + 1,
                    ((c.eax >> 14) & 4095) + 1
                ),
                "",
                format!("CPUID {leaf:08X}h; per cache instance"),
            ));
        }
    }
}
#[cfg(not(target_arch = "x86_64"))]
fn cpu_details(_: &mut Vec<Field>) {}

#[cfg(any(target_arch = "x86_64", test))]
fn cache_size(ebx: u32, ecx: u32) -> Option<u64> {
    // Hypervisors can supply CPUID leaves; every factor is architecturally
    // encoded as value - 1, and their product can reach 2^64.
    [
        ((ebx >> 22) & 1023) as u64 + 1,
        ((ebx >> 12) & 1023) as u64 + 1,
        (ebx & 4095) as u64 + 1,
        ecx as u64 + 1,
    ]
    .into_iter()
    .try_fold(1u64, u64::checked_mul)
}

pub struct Monitor {
    system: System,
    #[cfg(not(windows))]
    first: bool,
    #[cfg(windows)]
    cpu: windows::CpuMonitor,
    #[cfg(windows)]
    gpu: windows::GpuMonitor,
}
impl Monitor {
    pub fn new(inventory: &Inventory) -> Self {
        let mut system = System::new();
        system.refresh_cpu_all();
        system.refresh_memory();
        Self {
            #[cfg(windows)]
            cpu: windows::CpuMonitor::new(system.cpus().len()),
            system,
            #[cfg(not(windows))]
            first: true,
            #[cfg(windows)]
            gpu: windows::GpuMonitor::new(&inventory.adapters),
        }
    }
    pub fn sample(&mut self) -> Telemetry {
        #[cfg(not(windows))]
        self.system.refresh_cpu_all();
        #[cfg(not(windows))]
        self.system.refresh_memory();
        #[cfg(not(windows))]
        let usage = |value: f32, first: bool| {
            if first {
                Reading::missing(
                    "%",
                    "Windows CPU times / sysinfo",
                    "Waiting for two CPU-time samples",
                )
            } else {
                Reading::valid(value as f64, "%", "Windows CPU times / sysinfo")
            }
        };
        #[cfg(not(windows))]
        let cpu_usage = usage(self.system.global_cpu_usage(), self.first);
        #[cfg(not(windows))]
        let per_core_usage = self
            .system
            .cpus()
            .iter()
            .map(|cpu| usage(cpu.cpu_usage(), self.first))
            .collect();
        #[cfg(windows)]
        let (cpu_usage, per_core_usage) = self.cpu.sample();
        #[cfg(windows)]
        let cpu_frequency = windows::cpu_frequency(self.system.cpus().len());
        #[cfg(not(windows))]
        let cpu_frequency = Reading::missing(
            "MHz",
            "Windows processor power information",
            "This provider requires Windows",
        );
        #[cfg(not(windows))]
        let total = self.system.total_memory();
        #[cfg(not(windows))]
        let memory_total = if total == 0 {
            Reading::missing(
                "bytes",
                "GlobalMemoryStatusEx / sysinfo",
                "Windows did not report usable memory",
            )
        } else {
            Reading::valid(total as f64, "bytes", "GlobalMemoryStatusEx / sysinfo")
        };
        #[cfg(not(windows))]
        let memory_used = if total == 0 {
            Reading::missing(
                "bytes",
                "GlobalMemoryStatusEx / sysinfo",
                "Windows did not report usable memory",
            )
        } else {
            Reading::valid(
                self.system.used_memory() as f64,
                "bytes",
                "GlobalMemoryStatusEx / sysinfo",
            )
        };
        #[cfg(windows)]
        let (memory_used, memory_total) = windows::memory();
        #[cfg(not(windows))]
        {
            self.first = false;
        }
        Telemetry {
            timestamp_ms: now_ms(),
            cpu_usage,
            per_core_usage,
            memory_used,
            memory_total,
            cpu_frequency,
            #[cfg(windows)]
            gpus: self.gpu.sample(),
            #[cfg(not(windows))]
            gpus: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpuid_cache_geometry_cannot_wrap_into_a_valid_capacity() {
        // Eight ways, one partition, 64-byte lines, 64 sets = 32 KiB.
        assert_eq!(cache_size((7 << 22) | 63, 63), Some(32 * 1024));
        assert_eq!(cache_size(u32::MAX, u32::MAX), None);
    }
}
