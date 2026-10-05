use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Availability {
    Valid,
    Unsupported,
    PermissionRequired,
    Unavailable,
    Failed,
    Stale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Field {
    pub label: String,
    pub value: Option<String>,
    pub unit: String,
    pub source: String,
    pub state: Availability,
    pub detail: String,
    pub timestamp_ms: u64,
}
impl Field {
    pub fn valid(
        label: impl Into<String>,
        value: impl Into<String>,
        unit: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            value: Some(value.into()),
            unit: unit.into(),
            source: source.into(),
            state: Availability::Valid,
            detail: String::new(),
            timestamp_ms: now_ms(),
        }
    }
    pub fn missing(
        label: impl Into<String>,
        state: Availability,
        source: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            value: None,
            unit: String::new(),
            source: source.into(),
            state,
            detail: detail.into(),
            timestamp_ms: now_ms(),
        }
    }
    pub fn display(&self) -> String {
        match &self.value {
            Some(v) => {
                if self.unit.is_empty() {
                    v.clone()
                } else {
                    format!("{} {}", v, self.unit)
                }
            }
            None => match self.state {
                Availability::Unsupported => "Unsupported",
                Availability::PermissionRequired => "Permission required",
                Availability::Unavailable => "Unavailable",
                Availability::Failed => "Provider failed",
                Availability::Stale => "Stale",
                Availability::Valid => "Unavailable",
            }
            .into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reading {
    pub value: Option<f64>,
    pub unit: String,
    pub source: String,
    pub state: Availability,
    pub detail: String,
    pub timestamp_ms: u64,
}
impl Reading {
    pub fn valid(value: f64, unit: &str, source: &str) -> Self {
        if !value.is_finite() {
            return Self::missing(unit, source, "Provider returned a non-finite value");
        }
        Self {
            value: Some(value),
            unit: unit.into(),
            source: source.into(),
            state: Availability::Valid,
            detail: String::new(),
            timestamp_ms: now_ms(),
        }
    }
    pub fn missing(unit: &str, source: &str, detail: &str) -> Self {
        Self {
            value: None,
            unit: unit.into(),
            source: source.into(),
            state: Availability::Unavailable,
            detail: detail.into(),
            timestamp_ms: now_ms(),
        }
    }
    pub fn mark_stale(&mut self, now: u64) {
        if self.state == Availability::Valid && now.saturating_sub(self.timestamp_ms) > 5_000 {
            self.state = Availability::Stale;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Adapter {
    pub id: String,
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub fields: Vec<Field>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Drive {
    pub mount: String,
    pub name: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub cpu: Vec<Field>,
    pub memory: Vec<Field>,
    pub motherboard: Vec<Field>,
    pub os: Vec<Field>,
    pub adapters: Vec<Adapter>,
    pub drives: Vec<Drive>,
    pub diagnostics: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuSample {
    pub adapter_id: String,
    pub readings: Vec<(String, Reading)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Telemetry {
    pub timestamp_ms: u64,
    pub cpu_usage: Reading,
    pub per_core_usage: Vec<Reading>,
    pub memory_used: Reading,
    pub memory_total: Reading,
    pub cpu_frequency: Reading,
    pub gpus: Vec<GpuSample>,
}

pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut n = value as f64;
    let mut unit = 0;
    while n >= 1024.0 && unit < 4 {
        n /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{n:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_is_valid_and_missing_is_not_zero() {
        let zero = Reading::valid(0.0, "%", "test");
        assert_eq!(zero.state, Availability::Valid);
        assert_eq!(zero.value, Some(0.0));
        assert_eq!(Reading::missing("%", "test", "missing").value, None);
        assert_eq!(Reading::valid(f64::NAN, "%", "test").value, None);
    }
    #[test]
    fn stale_preserves_last_known_value() {
        let mut value = Reading::valid(42.0, "°C", "test");
        value.mark_stale(value.timestamp_ms + 5_001);
        assert_eq!(value.state, Availability::Stale);
        assert_eq!(value.value, Some(42.0));
    }
    #[test]
    fn binary_units_are_explicit() {
        assert_eq!(bytes(1_073_741_824), "1.00 GiB");
        assert_eq!(bytes(0), "0 B");
    }
}
