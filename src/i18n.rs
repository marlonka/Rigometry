//! Presentation-only localization. Provider identifiers and exported schemas stay stable.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    System,
    English,
    German,
    French,
    Spanish,
}

impl Language {
    pub const ALL: [Self; 5] = [
        Self::System,
        Self::English,
        Self::German,
        Self::French,
        Self::Spanish,
    ];

    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "system" => Some(Self::System),
            "en" => Some(Self::English),
            "de" => Some(Self::German),
            "fr" => Some(Self::French),
            "es" => Some(Self::Spanish),
            _ => None,
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Self::System => "System language",
            Self::English => "English",
            Self::German => "Deutsch",
            Self::French => "Français",
            Self::Spanish => "Español",
        }
    }

    pub fn resolve(self) -> Self {
        if self != Self::System {
            return self;
        }
        #[cfg(windows)]
        {
            // Primary language bits also cover regional variants such as fr-CA and es-MX.
            let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
            match id {
                0x07 => Self::German,
                0x0c => Self::French,
                0x0a => Self::Spanish,
                _ => Self::English,
            }
        }
        #[cfg(not(windows))]
        {
            Self::English
        }
    }

    pub fn text(self, source: &str) -> String {
        let language = self.resolve();
        let index = match language {
            Self::German => 0,
            Self::French => 1,
            Self::Spanish => 2,
            _ => return source.to_owned(),
        };
        catalog()
            .get(source)
            .map(|values| values[index].clone())
            .unwrap_or_else(|| source.to_owned())
    }

    pub fn format(self, source: &str, args: &[String]) -> String {
        render(&self.text(source), args)
    }

    /// Localize authored diagnostic templates, preserving opaque OS errors and device data.
    pub fn message(self, source: &str) -> String {
        self.message_inner(source, 0)
    }

    fn message_inner(self, source: &str, depth: usize) -> String {
        if self.resolve() == Self::English {
            return source.to_owned();
        }
        if catalog().contains_key(source) {
            return self.text(source);
        }
        if let Some(rest) = source.strip_prefix("Module ")
            && let Some((index, label)) = rest.split_once(' ')
            && index.parse::<usize>().is_ok()
        {
            let mut label = label.to_owned();
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            return format!(
                "{} · {}",
                self.format("Module {0}", &[index.into()]),
                self.text(&label)
            );
        }
        for pattern in patterns() {
            if let Some(mut args) = capture(pattern, source) {
                // Only known diagnostic arguments may contain authored messages.
                // File paths, device names and identifiers are always opaque.
                let nested: &[usize] = match *pattern {
                    "Export failed: {0}"
                    | "Memory readings: {0}"
                    | "SMBIOS parse: {0}"
                    | "CPU topology: {0}"
                    | "Hardware sampling failed during capture: {0}"
                    | "MFT fast path unavailable: {0}. Standard enumeration used; no elevation requested."
                    | "{0} exceed the u64 limit; reported size is a lower bound" => &[0],
                    "NVML status {0}: {1}"
                    | "NVML device {0} identification failed: {1}; incomplete enumeration cannot prove a unique PCI match" => {
                        &[1]
                    }
                    "Drive {0} ({1}) omitted: capacity is unavailable, not zero. {2}" => &[2],
                    "Firmware manufacturer: {0}; module manufacturer ID: {1}" => &[0],
                    _ => &[],
                };
                if depth < 4 {
                    for &index in nested {
                        args[index] = self.message_inner(&args[index], depth + 1);
                    }
                }
                return self.format(pattern, &args);
            }
        }
        if depth < 4 {
            // Scan diagnostics concatenate authored notes or prefix them with a path.
            if source.contains("; ") {
                return source
                    .split("; ")
                    .map(|part| self.message_inner(part, depth + 1))
                    .collect::<Vec<_>>()
                    .join("; ");
            }
            if let Some((prefix, detail)) = source.split_once(": ") {
                return format!("{prefix}: {}", self.message_inner(detail, depth + 1));
            }
        }
        source.to_owned()
    }

    pub fn decimal(self, value: f64, precision: usize) -> String {
        let value = format!("{value:.precision$}");
        if matches!(self.resolve(), Self::German | Self::French | Self::Spanish) {
            value.replace('.', ",")
        } else {
            value
        }
    }

    pub fn integer(self, value: u64) -> String {
        let separator = match self.resolve() {
            Self::German | Self::Spanish => '.',
            Self::French => '\u{202f}',
            _ => ',',
        };
        let digits = value.to_string();
        let mut out = String::new();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(separator);
            }
            out.push(ch);
        }
        out
    }

    pub fn bytes(self, value: u64) -> String {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
        let mut n = value as f64;
        let mut unit = 0;
        while n >= 1024. && unit < 4 {
            n /= 1024.;
            unit += 1;
        }
        if unit == 0 {
            format!("{} B", self.integer(value))
        } else {
            format!("{} {}", self.decimal(n, 2), UNITS[unit])
        }
    }
}

fn patterns() -> &'static Vec<&'static str> {
    static PATTERNS: OnceLock<Vec<&'static str>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let mut keys: Vec<_> = catalog()
            .keys()
            .filter(|key| key.contains("{0}"))
            .map(String::as_str)
            .collect();
        // Prefer specific templates to generic suffixes and diagnostic wrappers.
        keys.sort_by_key(|key| std::cmp::Reverse(key.len()));
        keys
    })
}

fn catalog() -> &'static BTreeMap<String, [String; 3]> {
    static CATALOG: OnceLock<BTreeMap<String, [String; 3]>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut catalog = BTreeMap::new();
        for source in [
            include_str!("i18n/messages.json"),
            include_str!("i18n/hardware.json"),
            include_str!("i18n/diagnostics.json"),
            include_str!("i18n/cli.json"),
        ] {
            let rows: Vec<[String; 4]> =
                serde_json::from_str(source).expect("valid bundled translations");
            for [key, de, fr, es] in rows {
                assert!(
                    catalog.insert(key.clone(), [de, fr, es]).is_none(),
                    "duplicate translation key: {key}"
                );
            }
        }
        catalog
    })
}

fn render(template: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        out.push_str(&remaining[..start]);
        let tail = &remaining[start..];
        if let Some(end) = tail.find('}')
            && let Ok(index) = tail[1..end].parse::<usize>()
            && let Some(value) = args.get(index)
        {
            out.push_str(value);
            remaining = &tail[end + 1..];
        } else {
            out.push('{');
            remaining = &tail[1..];
        }
    }
    out.push_str(remaining);
    out
}

fn capture(pattern: &str, source: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut pattern = pattern;
    let mut source = source;
    while let Some(start) = pattern.find('{') {
        let prefix = &pattern[..start];
        source = source.strip_prefix(prefix)?;
        let end = pattern[start..].find('}')? + start;
        if pattern[start + 1..end].parse::<usize>().ok()? != args.len() {
            return None;
        }
        pattern = &pattern[end + 1..];
        let separator = pattern.split('{').next()?;
        if separator.is_empty() && !pattern.is_empty() {
            return None;
        }
        let length = if separator.is_empty() {
            source.len()
        } else {
            source.find(separator)?
        };
        args.push(source[..length].to_owned());
        source = &source[length..];
    }
    (pattern == source).then_some(args)
}

pub fn set_context(ctx: &eframe::egui::Context, language: Language) {
    ctx.data_mut(|data| {
        data.insert_temp(
            eframe::egui::Id::new("interface-language"),
            language.resolve(),
        )
    });
}
pub fn language(ui: &eframe::egui::Ui) -> Language {
    ui.ctx()
        .data(|data| data.get_temp(eframe::egui::Id::new("interface-language")))
        .unwrap_or(Language::English)
}
pub fn text(ui: &eframe::egui::Ui, source: impl AsRef<str>) -> String {
    language(ui).text(source.as_ref())
}
pub fn message(ui: &eframe::egui::Ui, source: impl AsRef<str>) -> String {
    language(ui).message(source.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(text: &str) -> Vec<usize> {
        let mut values: Vec<_> = text
            .split('{')
            .skip(1)
            .map(|part| {
                part.split('}')
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .expect("numeric placeholder")
            })
            .collect();
        values.sort_unstable();
        values
    }
    #[test]
    fn translations_are_complete_and_preserve_all_arguments() {
        for (key, values) in catalog() {
            for value in values {
                assert!(!value.trim().is_empty(), "missing translation: {key}");
                assert_eq!(
                    placeholders(key),
                    placeholders(value),
                    "lost arguments: {key}"
                );
            }
        }
    }
    #[test]
    fn formatting_preserves_injected_paths_and_uses_decimal_commas() {
        assert_eq!(Language::German.decimal(12.5, 1), "12,5");
        assert_eq!(Language::French.integer(1234567), "1\u{202f}234\u{202f}567");
        assert_eq!(Language::Spanish.bytes(1073741824), "1,00 GiB");
        assert_eq!(Language::English.bytes(1073741824), "1.00 GiB");
        assert_eq!(
            render("{1}: {0}", &["C:\\{1}\\資料.csv".into(), "Saved".into()]),
            "Saved: C:\\{1}\\資料.csv"
        );
        assert_eq!(
            capture("Saved {0}", "Saved C:\\{1}\\資料.csv"),
            Some(vec!["C:\\{1}\\資料.csv".into()])
        );
    }

    #[test]
    fn composed_diagnostics_translate_without_modifying_opaque_values() {
        assert_eq!(Language::German.message("Export failed: File already exists; choose a new file name. Existing files are never replaced."), "Export fehlgeschlagen: Datei existiert bereits; einen neuen Dateinamen wählen. Vorhandene Dateien werden niemals ersetzt.");
        assert_eq!(
            Language::Spanish.message("Module 2 configured transfer rate"),
            "Módulo 2 · Tasa de transferencia configurada"
        );
        assert_eq!(
            Language::French.message("Saved C:\\Unavailable\\{0}.csv"),
            "Enregistré : C:\\Unavailable\\{0}.csv"
        );
        assert_eq!(
            Language::German.message("C:\\資料: MFT index exceeded its 512 MiB memory budget"),
            "C:\\資料: MFT-Index überschritt sein Speicherbudget von 512 MiB"
        );
        assert_eq!(
            Language::French.message("Sparse; NTFS compressed"),
            "Fichier creux; Compressé NTFS"
        );
        assert_eq!(
            Language::Spanish
                .message("NVML status 3: not supported by this GPU/driver or API symbol absent"),
            "Estado NVML 3: no compatible con esta GPU/controlador o falta el símbolo de API"
        );
    }

    #[test]
    fn static_interface_labels_have_a_catalog_entry() {
        // Check the actual presentation call sites, including multiline calls.
        // IDs, provider keys and file names intentionally do not use `tx`.
        for call in include_str!("ui.rs").split("tx(").skip(1) {
            let Some(source) = call.trim_start().strip_prefix("ui,") else {
                continue;
            };
            let source = source.trim_start();
            if !source.starts_with('"') {
                continue;
            }
            let mut escaped = false;
            let end = source
                .char_indices()
                .skip(1)
                .find_map(|(index, ch)| {
                    if escaped {
                        escaped = false;
                        return None;
                    }
                    if ch == '\\' {
                        escaped = true;
                        return None;
                    }
                    (ch == '"').then_some(index)
                })
                .unwrap();
            let key: String = serde_json::from_str(&source[..=end]).unwrap();
            assert!(
                key.is_empty()
                    || ["Windows", "BIOS", "−120 s"].contains(&key.as_str())
                    || catalog().contains_key(&key),
                "untranslated UI label: {key}"
            );
        }
    }
}
