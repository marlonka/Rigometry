//! macOS hardware discovery: `sysctl` for the processor, `system_profiler` for the
//! Mac, memory and graphics, and IOKit accelerator statistics for live GPU load.
//! Parsers are platform-independent so their tests run on every host.
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use crate::model::{Adapter, Availability, Field, GpuSample, Inventory, Reading, bytes};
use serde_json::Value;
use std::collections::BTreeMap;

const PROFILER: &str = "system_profiler (SPHardwareDataType)";
const DISPLAYS: &str = "system_profiler (SPDisplaysDataType)";
const MEMORY: &str = "system_profiler (SPMemoryDataType)";
const IOREG: &str = "IOKit IOAccelerator PerformanceStatistics";

#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::ptr::null_mut;

    unsafe extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }

    fn raw(name: &str) -> Option<Vec<u8>> {
        let name = CString::new(name).ok()?;
        let mut length = 0usize;
        // SAFETY: a null buffer asks only for the value's size.
        if unsafe { sysctlbyname(name.as_ptr(), null_mut(), &mut length, null_mut(), 0) } != 0
            || length == 0
        {
            return None;
        }
        let mut buffer = vec![0u8; length];
        // SAFETY: `buffer` holds `length` writable bytes; sysctl updates `length`.
        if unsafe {
            sysctlbyname(
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut length,
                null_mut(),
                0,
            )
        } != 0
        {
            return None;
        }
        buffer.truncate(length);
        Some(buffer)
    }

    pub fn string(name: &str) -> Option<String> {
        let value = raw(name)?;
        let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
        let text = String::from_utf8_lossy(&value[..end]).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    pub fn number(name: &str) -> Option<u64> {
        let value = raw(name)?;
        match value.len() {
            4 => Some(u32::from_ne_bytes(value.try_into().ok()?) as u64),
            8 => Some(u64::from_ne_bytes(value.try_into().ok()?)),
            _ => None,
        }
    }
}

/// Runs a system tool for its output. A tool that hangs is stopped after
/// 30 seconds, so it cannot stall discovery or sampling for good.
#[cfg(target_os = "macos")]
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    use std::{
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    const TIMEOUT: Duration = Duration::from_secs(30);
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;
    // Read while waiting: a full pipe would otherwise block the tool.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map(|_| output)
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} did not finish within {} s",
                    TIMEOUT.as_secs()
                ));
            }
            Err(error) => return Err(format!("{program}: {error}")),
        }
    };
    let output = reader
        .join()
        .map_err(|_| format!("{program}: output reader failed"))?
        .map_err(|error| format!("{program}: {error}"))?;
    if !status.success() {
        return Err(format!("{program} exited with {status}"));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

#[cfg(target_os = "macos")]
pub(super) fn discover(result: &mut Inventory) {
    cpu(&mut result.cpu);
    if let Some(size) = sys::number("hw.memsize") {
        result.memory.insert(
            0,
            Field::valid("Installed capacity", bytes(size), "", "sysctl hw.memsize"),
        );
    }
    // Absolute path: never resolve a system tool through PATH.
    match run(
        "/usr/sbin/system_profiler",
        &[
            "-json",
            "-detailLevel",
            "mini",
            "SPHardwareDataType",
            "SPMemoryDataType",
            "SPDisplaysDataType",
        ],
    )
    .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|error| error.to_string()))
    {
        Ok(profile) => {
            hardware(&profile, &mut result.motherboard);
            memory(&profile, &mut result.memory);
            result.adapters = adapters(&profile);
        }
        Err(error) => result
            .diagnostics
            .push(format!("System profiler unavailable: {error}")),
    }
    if result.adapters.is_empty() {
        result
            .diagnostics
            .push("macOS reported no graphics processor.".into());
    }
    result.diagnostics.push("macOS discovery is read-only: sysctl, system_profiler and IOKit registry properties, without administrator rights. Serial numbers and hardware UUIDs are not read into reports.".into());
}

/// Processor topology and caches from `sysctl`. Intel Macs also get CPUID details.
#[cfg(target_os = "macos")]
fn cpu(fields: &mut Vec<Field>) {
    let levels = sys::number("hw.nperflevels").unwrap_or(0).min(4);
    for level in 0..levels {
        let prefix = format!("hw.perflevel{level}");
        let name = sys::string(&format!("{prefix}.name"));
        let label = match (level, name.as_deref()) {
            (_, Some("Performance")) | (0, None) => "Performance cores",
            (_, Some("Efficiency")) | (1, None) => "Efficiency cores",
            _ => "Additional cores",
        };
        if let Some(cores) = sys::number(&format!("{prefix}.physicalcpu")) {
            fields.push(Field::valid(
                label,
                cores.to_string(),
                "",
                format!("sysctl {prefix}.physicalcpu"),
            ));
        }
        if cfg!(target_arch = "x86_64") {
            continue;
        }
        let kind = if label == "Efficiency cores" {
            "efficiency"
        } else {
            "performance"
        };
        for (cache, key) in [
            ("L1 data cache", "l1dcachesize"),
            ("L1 instruction cache", "l1icachesize"),
        ] {
            if let Some(size) = sys::number(&format!("{prefix}.{key}")).filter(|&v| v > 0) {
                fields.push(Field::valid(
                    format!("{cache} ({kind})"),
                    format!("{} · per core", bytes(size)),
                    "",
                    format!("sysctl {prefix}.{key}"),
                ));
            }
        }
        if let Some(size) = sys::number(&format!("{prefix}.l2cachesize")).filter(|&v| v > 0) {
            let shared = sys::number(&format!("{prefix}.cpusperl2")).unwrap_or(0);
            let value = if shared > 1 {
                format!("{} · shared by {shared} cores", bytes(size))
            } else {
                bytes(size)
            };
            fields.push(Field::valid(
                format!("L2 cache ({kind})"),
                value,
                "",
                format!("sysctl {prefix}.l2cachesize"),
            ));
        }
    }
    if cfg!(target_arch = "aarch64") {
        let mut features = Vec::new();
        for (key, name) in ARM_FEATURES {
            if sys::number(key).unwrap_or(0) != 0 {
                features.push(*name);
            }
        }
        if !features.is_empty() {
            fields.push(Field::valid(
                "Instruction sets (hardware)",
                features.join(" · "),
                "",
                "sysctl hw.optional.arm; OS enablement implied",
            ));
        }
        if let Some(line) = sys::number("hw.cachelinesize") {
            fields.push(Field::valid(
                "Cache line",
                line.to_string(),
                "bytes",
                "sysctl hw.cachelinesize",
            ));
        }
    }
}

const ARM_FEATURES: &[(&str, &str)] = &[
    ("hw.optional.AdvSIMD", "NEON"),
    ("hw.optional.arm.FEAT_FP16", "FP16"),
    ("hw.optional.arm.FEAT_DotProd", "DotProd"),
    ("hw.optional.arm.FEAT_I8MM", "I8MM"),
    ("hw.optional.arm.FEAT_BF16", "BF16"),
    ("hw.optional.arm.FEAT_AES", "AES"),
    ("hw.optional.arm.FEAT_SHA256", "SHA-256"),
    ("hw.optional.arm.FEAT_SHA512", "SHA-512"),
    ("hw.optional.arm.FEAT_SHA3", "SHA-3"),
    ("hw.optional.armv8_crc32", "CRC32"),
    ("hw.optional.arm.FEAT_LSE", "LSE atomics"),
    ("hw.optional.arm.FEAT_SME", "SME"),
    ("hw.optional.arm.FEAT_SME2", "SME2"),
];

/// The processor name as macOS reports it, such as "Apple M3 Pro".
#[cfg(all(target_os = "macos", not(target_arch = "x86_64")))]
pub(super) fn brand() -> Option<String> {
    sys::string("machdep.cpu.brand_string")
}

fn section<'a>(profile: &'a Value, name: &str) -> &'a [Value] {
    profile
        .get(name)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn text<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    match item.get(key)? {
        Value::String(value) => Some(value.trim()).filter(|v| !v.is_empty()),
        _ => None,
    }
}

/// `system_profiler` encodes enumerations as keys such as `spdisplays_metal3`.
fn pretty(value: &str) -> String {
    let value = ["sppci_vendor_", "spdisplays_", "sppci_", "ppci_"]
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .unwrap_or(value);
    match value.to_ascii_lowercase().as_str() {
        "apple" => "Apple".into(),
        "amd" | "ati" => "AMD".into(),
        "intel" => "Intel".into(),
        "nvidia" => "NVIDIA".into(),
        "builtin" => "Built-in".into(),
        "pcie_device" | "pcie" => "PCI Express".into(),
        "thunderbolt" | "thunderbolt_device" => "Thunderbolt (eGPU)".into(),
        "supported" | "yes" => "Yes".into(),
        "not_supported" | "no" => "No".into(),
        lower if lower.starts_with("metal") => {
            let version = lower.trim_start_matches("metal");
            if version.is_empty() {
                "Metal".into()
            } else {
                format!("Metal {version}")
            }
        }
        _ => value.replace('_', " "),
    }
}

fn hardware(profile: &Value, fields: &mut Vec<Field>) {
    let Some(item) = section(profile, "SPHardwareDataType").first() else {
        return;
    };
    fields.push(Field::valid("Manufacturer", "Apple", "", PROFILER));
    let model = match (text(item, "machine_name"), text(item, "machine_model")) {
        (Some(name), Some(id)) => format!("{name} ({id})"),
        (Some(name), None) => name.into(),
        (None, Some(id)) => id.into(),
        (None, None) => return,
    };
    fields.push(Field::valid("Model", model, "", PROFILER));
    for (label, key) in [
        ("Model number", "model_number"),
        ("Chip", "chip_type"),
        ("Processor", "cpu_type"),
        ("Processor speed", "current_processor_speed"),
        ("BIOS version", "boot_rom_version"),
        ("OS loader version", "os_loader_version"),
        ("SMC version", "SMC_version_system"),
    ] {
        if let Some(value) = text(item, key) {
            fields.push(Field::valid(label, value, "", PROFILER));
        }
    }
}

fn memory(profile: &Value, fields: &mut Vec<Field>) {
    let Some(item) = section(profile, "SPMemoryDataType").first() else {
        return;
    };
    // Apple silicon reports one unified package; Intel Macs list each slot.
    let modules: Vec<&Value> = match item.get("_items").and_then(Value::as_array) {
        Some(slots) => slots
            .iter()
            .filter(|slot| text(slot, "dimm_size").is_some_and(|size| size != "Empty"))
            .collect(),
        None => vec![item],
    };
    if let Some(first) = modules.first() {
        if let Some(kind) = text(first, "dimm_type") {
            fields.push(Field::valid("Memory type", kind, "", MEMORY));
        }
        if let Some(maker) = text(first, "dimm_manufacturer") {
            fields.push(Field::valid("Module manufacturer", maker, "", MEMORY));
        }
        if let Some(speed) = text(first, "dimm_speed") {
            fields.push(Field::valid("Configured speed", speed, "", MEMORY));
        }
    }
    if item.get("_items").is_some() {
        fields.push(Field::valid(
            "Populated modules",
            modules.len().to_string(),
            "",
            MEMORY,
        ));
    } else {
        fields.push(Field::valid(
            "Memory architecture",
            "Unified memory, shared by CPU and GPU",
            "",
            MEMORY,
        ));
    }
    if let Some(upgradeable) = text(item, "is_memory_upgradeable") {
        fields.push(Field::valid("Upgradeable", pretty(upgradeable), "", MEMORY));
    }
}

fn vendor_id(item: &Value) -> u32 {
    if let Some(id) = text(item, "spdisplays_vendor-id")
        .and_then(|id| u32::from_str_radix(id.trim_start_matches("0x"), 16).ok())
    {
        return id;
    }
    let vendor = text(item, "spdisplays_vendor").unwrap_or_default();
    let model = text(item, "sppci_model").unwrap_or_default();
    match pretty(vendor).as_str() {
        "Apple" => 0x106b,
        "AMD" => 0x1002,
        "Intel" => 0x8086,
        "NVIDIA" => 0x10de,
        _ if model.starts_with("Apple") => 0x106b,
        _ if model.contains("Radeon") => 0x1002,
        _ if model.starts_with("Intel") => 0x8086,
        _ => 0,
    }
}

fn adapters(profile: &Value) -> Vec<Adapter> {
    section(profile, "SPDisplaysDataType")
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let name = text(item, "sppci_model").or_else(|| text(item, "_name"))?;
            let vendor = vendor_id(item);
            let device = text(item, "spdisplays_device-id")
                .and_then(|id| u32::from_str_radix(id.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0);
            let mut fields = vec![Field::valid("Model", name, "", DISPLAYS)];
            if let Some(vendor) = text(item, "spdisplays_vendor") {
                fields.push(Field::valid("Vendor", pretty(vendor), "", DISPLAYS));
            }
            if let Some(cores) = text(item, "sppci_cores") {
                fields.push(Field::valid("GPU cores", cores, "", DISPLAYS));
            }
            if let Some(metal) = text(item, "spdisplays_mtlgpufamilysupport") {
                fields.push(Field::valid("Metal support", pretty(metal), "", DISPLAYS));
            }
            if let Some(bus) = text(item, "sppci_bus") {
                fields.push(Field::valid("Bus", pretty(bus), "", DISPLAYS));
            }
            match (
                text(item, "spdisplays_vram"),
                text(item, "spdisplays_vram_shared"),
            ) {
                (Some(vram), _) => fields.push(Field::valid("Video memory", vram, "", DISPLAYS)),
                (None, Some(shared)) => fields.push(Field::valid(
                    "Video memory",
                    format!("{shared} (shared)"),
                    "",
                    DISPLAYS,
                )),
                (None, None) if vendor == 0x106b => fields.push(Field::valid(
                    "Video memory",
                    "Unified memory, shared with the CPU",
                    "",
                    DISPLAYS,
                )),
                _ => {}
            }
            if let Some(width) = text(item, "spdisplays_pcie_width") {
                fields.push(Field::valid("PCIe width", width, "", DISPLAYS));
            }
            if device != 0 {
                fields.push(Field::valid(
                    "Vendor / device ID",
                    format!("{vendor:04X} / {device:04X}"),
                    "",
                    DISPLAYS,
                ));
            }
            let displays: Vec<String> = item
                .get("spdisplays_ndrvs")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|display| {
                            let name = text(display, "_name")?;
                            Some(
                                match text(display, "_spdisplays_resolution")
                                    .or_else(|| text(display, "spdisplays_resolution"))
                                {
                                    Some(mode) => format!("{name} · {mode}"),
                                    None => name.to_owned(),
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !displays.is_empty() {
                fields.push(Field::valid(
                    "Connected displays",
                    displays.join("; "),
                    "",
                    DISPLAYS,
                ));
            }
            Some(Adapter {
                id: format!("macos-gpu-{index}"),
                name: name.to_owned(),
                vendor_id: vendor,
                device_id: device,
                fields,
            })
        })
        .collect()
}

/// One IOAccelerator registry entry from `ioreg -r -d 1 -w 0 -c IOAccelerator`.
#[derive(Debug, Default, PartialEq)]
struct Accelerator {
    class: String,
    statistics: BTreeMap<String, f64>,
}

fn parse_ioreg(output: &str) -> Vec<Accelerator> {
    let mut accelerators: Vec<Accelerator> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("+-o ") {
            let class = rest.split_whitespace().next().unwrap_or_default();
            accelerators.push(Accelerator {
                class: class.to_owned(),
                ..Default::default()
            });
            continue;
        }
        let Some(current) = accelerators.last_mut() else {
            continue;
        };
        if let Some(body) = line.strip_prefix("\"PerformanceStatistics\" = {") {
            let body = body.strip_suffix('}').unwrap_or(body);
            // Pairs are `"key"=value`; keys may contain commas, values never quotes.
            let mut rest = body;
            while let Some(start) = rest.find('"') {
                let after = &rest[start + 1..];
                let Some(end) = after.find("\"=") else {
                    break;
                };
                let key = &after[..end];
                let value_text = &after[end + 2..];
                let value_end = value_text.find(",\"").unwrap_or(value_text.len());
                if let Ok(value) = value_text[..value_end].trim().parse::<f64>() {
                    current.statistics.insert(key.to_owned(), value);
                }
                rest = &value_text[value_end..];
            }
        }
    }
    accelerators
}

/// Registry classes name the driver family, which is how adapters are paired.
/// Case-insensitively: Apple's Kepler driver class is `nvAccelerator`.
fn class_vendor(class: &str) -> u32 {
    let class = class.to_ascii_lowercase();
    if class.starts_with("agx") {
        0x106b
    } else if class.starts_with("intel") {
        0x8086
    } else if class.starts_with("amd") || class.starts_with("ati") || class.contains("radeon") {
        0x1002
    } else if class.starts_with("nv") || class.starts_with("geforce") {
        0x10de
    } else {
        0
    }
}

const SENSORS: &[(&str, &str, &[&str])] = &[
    (
        "Utilization",
        "%",
        &["Device Utilization %", "GPU Activity(%)"],
    ),
    ("Renderer utilization", "%", &["Renderer Utilization %"]),
    ("Tiler utilization", "%", &["Tiler Utilization %"]),
    (
        "GPU memory in use",
        "bytes",
        &["In use system memory", "vramUsedBytes"],
    ),
    ("Temperature", "°C", &["Temperature(C)"]),
    ("Core clock", "MHz", &["Core Clock(MHz)"]),
    ("Memory clock", "MHz", &["Memory Clock(MHz)"]),
    ("Power", "W", &["Total Power(W)"]),
    ("Fan", "%", &["Fan Speed(%)"]),
];

fn readings(adapter: &Adapter, accelerators: &[Accelerator]) -> Vec<(String, Reading)> {
    let candidates: Vec<&Accelerator> = accelerators
        .iter()
        .filter(|a| class_vendor(&a.class) == adapter.vendor_id)
        .collect();
    // A lone accelerator of unknown family may belong to the adapter; one of
    // another vendor never does (an unplugged eGPU must not show the iGPU's load).
    let matched = match (candidates.as_slice(), accelerators) {
        ([one], _) => Some(*one),
        ([], [only]) if adapter.vendor_id == 0 || class_vendor(&only.class) == 0 => Some(only),
        _ => None,
    };
    SENSORS
        .iter()
        .filter_map(|(label, unit, keys)| {
            let value = matched.and_then(|a| keys.iter().find_map(|k| a.statistics.get(*k)));
            let reading = match (value, matched) {
                (Some(value), _) => {
                    let mut reading = Reading::valid(*value, unit, IOREG);
                    if *label == "Utilization" {
                        reading.detail =
                            "Share of time the GPU was busy, as reported by its macOS driver."
                                .into();
                    }
                    reading
                }
                // Keep the two charted readings even when the driver lacks them.
                (None, _) if *label == "Utilization" || *label == "Temperature" => {
                    let mut reading = Reading::missing(
                        unit,
                        IOREG,
                        if matched.is_none() {
                            "No IOAccelerator statistics matched this graphics processor"
                        } else if adapter.vendor_id == 0x106b {
                            "macOS exposes Apple silicon GPU temperature only to privileged tools such as powermetrics; none is bundled."
                        } else {
                            "The macOS driver does not publish this reading"
                        },
                    );
                    reading.state = Availability::Unsupported;
                    reading
                }
                (None, _) => return None,
            };
            Some(((*label).into(), reading))
        })
        .collect()
}

pub(super) struct GpuMonitor {
    adapters: Vec<Adapter>,
}

impl GpuMonitor {
    pub(super) fn new(adapters: &[Adapter]) -> Self {
        Self {
            adapters: adapters.to_vec(),
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn sample(&mut self) -> Vec<GpuSample> {
        if self.adapters.is_empty() {
            return Vec::new();
        }
        let accelerators = run(
            "/usr/sbin/ioreg",
            &["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"],
        )
        .map(|text| parse_ioreg(&text));
        self.adapters
            .iter()
            .map(|adapter| GpuSample {
                adapter_id: adapter.id.clone(),
                readings: match &accelerators {
                    Ok(list) => readings(adapter, list),
                    Err(error) => vec![("Utilization".into(), Reading::missing("%", IOREG, error))],
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE_SILICON: &str = r#"{
      "SPDisplaysDataType": [{
        "_name": "Apple M3 Pro",
        "spdisplays_mtlgpufamilysupport": "spdisplays_metal3",
        "spdisplays_ndrvs": [{"_name": "Color LCD", "_spdisplays_resolution": "1512 x 982 @ 120.00Hz"}],
        "spdisplays_vendor": "sppci_vendor_Apple",
        "sppci_bus": "spdisplays_builtin",
        "sppci_cores": "18",
        "sppci_device_type": "spdisplays_gpu",
        "sppci_model": "Apple M3 Pro"
      }],
      "SPHardwareDataType": [{
        "_name": "hardware_overview",
        "boot_rom_version": "11881.1.1",
        "chip_type": "Apple M3 Pro",
        "machine_model": "Mac15,6",
        "machine_name": "MacBook Pro",
        "physical_memory": "18 GB",
        "serial_number": "MUST-NOT-APPEAR",
        "platform_UUID": "MUST-NOT-APPEAR"
      }],
      "SPMemoryDataType": [{"_name": "Memory", "SPMemoryDataType": "18 GB", "dimm_manufacturer": "Micron", "dimm_type": "LPDDR5"}]
    }"#;

    const INTEL_MAC: &str = r#"{
      "SPDisplaysDataType": [
        {"_name": "Intel UHD Graphics 630", "spdisplays_device-id": "0x3e9b", "spdisplays_vendor": "Intel", "spdisplays_vram_shared": "1536 MB", "sppci_bus": "spdisplays_builtin", "sppci_model": "Intel UHD Graphics 630"},
        {"_name": "AMD Radeon Pro 5500M", "spdisplays_device-id": "0x7340", "spdisplays_mtlgpufamilysupport": "spdisplays_metal3", "spdisplays_pcie_width": "x8", "spdisplays_vendor": "sppci_vendor_amd", "spdisplays_vram": "4 GB", "sppci_bus": "spdisplays_pcie_device", "sppci_model": "AMD Radeon Pro 5500M"}
      ],
      "SPMemoryDataType": [{"_name": "Memory Slots", "is_memory_upgradeable": "No", "_items": [
        {"_name": "BANK 0/ChannelA-DIMM0", "dimm_manufacturer": "Micron", "dimm_size": "8 GB", "dimm_speed": "2667 MHz", "dimm_type": "DDR4"},
        {"_name": "BANK 2/ChannelB-DIMM0", "dimm_manufacturer": "Micron", "dimm_size": "8 GB", "dimm_speed": "2667 MHz", "dimm_type": "DDR4"}
      ]}]
    }"#;

    const IOREG_OUTPUT: &str = r#"+-o AGXAcceleratorG15X  <class AGXAcceleratorG15X, id 0x1000004a8, registered, matched, active, busy 0 (0 ms), retain 63>
    {
      "IOClass" = "AGXAcceleratorG15X"
      "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=2293760000,"Tiler Utilization %"=7,"recoveryCount"=0,"Renderer Utilization %"=11,"Device Utilization %"=12,"In use system memory"=412041216}
      "model" = "Apple M3 Pro"
      "gpu-core-count" = 18
    }
"#;

    const IOREG_INTEL_MAC: &str = r#"+-o IntelAccelerator  <class IntelAccelerator, id 0x100000441, registered>
    {
      "PerformanceStatistics" = {"Device Utilization %"=3,"GPU Activity(%)"=3}
    }
+-o AMDRadeonX6000_AMDNavi14GraphicsAccelerator  <class AMDRadeonX6000_AMDNavi14GraphicsAccelerator, id 0x100000672>
    {
      "PerformanceStatistics" = {"Temperature(C)"=48,"Fan Speed(%)"=0,"Core Clock(MHz)"=300,"Memory Clock(MHz)"=100,"VRAM,Used Bytes"=1024,"Device Utilization %"=21,"Total Power(W)"=5}
    }
"#;

    fn field<'a>(fields: &'a [Field], label: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|f| f.label == label)
            .and_then(|f| f.value.as_deref())
    }

    #[test]
    fn apple_silicon_profile_names_the_mac_chip_gpu_and_unified_memory() {
        let profile: Value = serde_json::from_str(APPLE_SILICON).unwrap();
        let mut board = Vec::new();
        hardware(&profile, &mut board);
        assert_eq!(field(&board, "Manufacturer"), Some("Apple"));
        assert_eq!(field(&board, "Model"), Some("MacBook Pro (Mac15,6)"));
        assert_eq!(field(&board, "BIOS version"), Some("11881.1.1"));
        let everything = format!("{board:?}");
        assert!(
            !everything.contains("MUST-NOT-APPEAR"),
            "identifiers leaked"
        );

        let mut memory_fields = Vec::new();
        memory(&profile, &mut memory_fields);
        assert_eq!(field(&memory_fields, "Memory type"), Some("LPDDR5"));
        assert!(field(&memory_fields, "Memory architecture").is_some());

        let gpus = adapters(&profile);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Apple M3 Pro");
        assert_eq!(gpus[0].vendor_id, 0x106b);
        assert_eq!(field(&gpus[0].fields, "GPU cores"), Some("18"));
        assert_eq!(field(&gpus[0].fields, "Metal support"), Some("Metal 3"));
        assert_eq!(field(&gpus[0].fields, "Bus"), Some("Built-in"));
        assert_eq!(
            field(&gpus[0].fields, "Connected displays"),
            Some("Color LCD · 1512 x 982 @ 120.00Hz")
        );
    }

    #[test]
    fn intel_mac_profile_lists_both_gpus_and_memory_slots() {
        let profile: Value = serde_json::from_str(INTEL_MAC).unwrap();
        let gpus = adapters(&profile);
        assert_eq!(
            gpus.iter().map(|g| g.vendor_id).collect::<Vec<_>>(),
            [0x8086, 0x1002]
        );
        assert_eq!(
            field(&gpus[0].fields, "Video memory"),
            Some("1536 MB (shared)")
        );
        assert_eq!(field(&gpus[1].fields, "Video memory"), Some("4 GB"));
        assert_eq!(
            field(&gpus[1].fields, "Vendor / device ID"),
            Some("1002 / 7340")
        );
        let mut memory_fields = Vec::new();
        memory(&profile, &mut memory_fields);
        assert_eq!(field(&memory_fields, "Populated modules"), Some("2"));
        assert_eq!(field(&memory_fields, "Configured speed"), Some("2667 MHz"));
        assert_eq!(field(&memory_fields, "Upgradeable"), Some("No"));
    }

    #[test]
    fn accelerator_statistics_become_gpu_readings() {
        let profile: Value = serde_json::from_str(APPLE_SILICON).unwrap();
        let gpu = &adapters(&profile)[0];
        let accelerators = parse_ioreg(IOREG_OUTPUT);
        assert_eq!(accelerators.len(), 1);
        let values: BTreeMap<_, _> = readings(gpu, &accelerators).into_iter().collect();
        assert_eq!(values["Utilization"].value, Some(12.0));
        assert_eq!(values["Renderer utilization"].value, Some(11.0));
        assert_eq!(values["GPU memory in use"].value, Some(412041216.0));
        // Unknown is not zero: Apple silicon publishes no GPU temperature here.
        assert_eq!(values["Temperature"].value, None);
        assert_eq!(values["Temperature"].state, Availability::Unsupported);
        assert!(!values.contains_key("Fan"));
    }

    #[test]
    fn dual_gpu_macs_pair_statistics_by_driver_family() {
        let profile: Value = serde_json::from_str(INTEL_MAC).unwrap();
        let gpus = adapters(&profile);
        let accelerators = parse_ioreg(IOREG_INTEL_MAC);
        assert_eq!(accelerators.len(), 2);
        let intel: BTreeMap<_, _> = readings(&gpus[0], &accelerators).into_iter().collect();
        let amd: BTreeMap<_, _> = readings(&gpus[1], &accelerators).into_iter().collect();
        assert_eq!(intel["Utilization"].value, Some(3.0));
        assert_eq!(amd["Utilization"].value, Some(21.0));
        assert_eq!(amd["Temperature"].value, Some(48.0));
        assert_eq!(amd["Core clock"].value, Some(300.0));
        assert_eq!(amd["Power"].value, Some(5.0));
        // With the AMD GPU's statistics gone (an unplugged eGPU), the one
        // remaining accelerator is Intel's and must not become the AMD reading.
        let intel_only: Vec<_> = accelerators
            .into_iter()
            .filter(|a| class_vendor(&a.class) == 0x8086)
            .collect();
        let amd: BTreeMap<_, _> = readings(&gpus[1], &intel_only).into_iter().collect();
        assert_eq!(amd["Utilization"].value, None);
        assert_eq!(class_vendor("nvAccelerator"), 0x10de);
    }

    #[test]
    fn missing_or_malformed_profiles_report_nothing_instead_of_guessing() {
        let empty: Value = serde_json::from_str("{}").unwrap();
        assert!(adapters(&empty).is_empty());
        let mut fields = Vec::new();
        hardware(&empty, &mut fields);
        memory(&empty, &mut fields);
        assert!(fields.is_empty());
        assert!(parse_ioreg("garbage\n\"PerformanceStatistics\" = {").is_empty());
    }
}
