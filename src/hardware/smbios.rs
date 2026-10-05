//! Defensive parser for the public SMBIOS structures returned by Windows.
use crate::model::{Availability, Field, Inventory, bytes};

const MAX_RECORDS: usize = 4096;
const MAX_STRING_BYTES: usize = 1024 * 1024;

fn firmware_placeholder(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "unknown"
            | "not specified"
            | "not available"
            | "to be filled by o.e.m."
            | "default string"
            | "none"
            | "x.x"
            | "x.x.x"
    )
}

fn module_manufacturer(record: &[u8], strings: &[&[u8]], label: &str, source: &str) -> Field {
    let name = string_at(record, 23, strings);
    if let Some(name) = name.as_ref().filter(|name| !firmware_placeholder(name)) {
        return Field::valid(label, name, "", source);
    }
    // SMBIOS type 17, offset 2Ch: SPD module ID in byte order, not the
    // DRAM-chip manufacturer. JEDEC JEP106 bank five, code EF = Team Group.
    // Only this verified fallback is decoded; unknown IDs are never guessed.
    let id = record.get(44..46);
    if id == Some(&[0x04, 0xef][..]) {
        let mut field = Field::valid(label, "Team Group Inc.", "", source);
        field.detail = "Module manufacturer ID 04 EF (JEDEC bank 5, code EF); firmware manufacturer name missing".into();
        return field;
    }
    Field::missing(
        label,
        Availability::Unavailable,
        source,
        format!(
            "Firmware manufacturer: {}; module manufacturer ID: {}",
            name.as_deref().unwrap_or("not supplied"),
            id.map(|bytes| format!("{:02X} {:02X}", bytes[0], bytes[1]))
                .unwrap_or_else(|| "not supplied".into())
        ),
    )
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
fn string_at(data: &[u8], offset: usize, strings: &[&[u8]]) -> Option<String> {
    let idx = *data.get(offset)? as usize;
    let value = String::from_utf8_lossy(strings.get(idx.checked_sub(1)?)?)
        .trim()
        .to_string();
    if value.is_empty() { None } else { Some(value) }
}

pub(super) fn parse(raw: &[u8], result: &mut Inventory) -> Result<(), String> {
    // Do not publish apparently valid fragments from a malformed table. Bound
    // expansion into fields as well as the native byte buffer itself.
    let mut parsed = Inventory::default();
    parse_table(raw, &mut parsed)?;
    result.cpu.extend(parsed.cpu);
    result.memory.extend(parsed.memory);
    result.motherboard.extend(parsed.motherboard);
    Ok(())
}

fn parse_table(raw: &[u8], result: &mut Inventory) -> Result<(), String> {
    if raw.len() < 8 {
        return Err("Raw SMBIOS header truncated".into());
    }
    if raw.len() > 32 * 1024 * 1024 {
        return Err("SMBIOS table exceeds safety bound".into());
    }
    let length = u32_at(raw, 4).ok_or("Missing SMBIOS length")? as usize;
    let data = raw
        .get(8..8usize.checked_add(length).ok_or("SMBIOS length overflow")?)
        .ok_or("SMBIOS table length exceeds returned bytes")?;
    let source = format!(
        "SMBIOS {}.{} / GetSystemFirmwareTable; firmware-reported",
        raw[1], raw[2]
    );
    let mut pos = 0;
    let mut module = 0;
    let mut installed = 0u64;
    let mut unknown_capacity = false;
    let mut records = 0;
    let mut string_bytes = 0;
    while pos < data.len() {
        records += 1;
        if records > MAX_RECORDS {
            return Err("SMBIOS record count exceeds safety bound".into());
        }
        let header = data
            .get(pos..pos + 4)
            .ok_or("Truncated SMBIOS structure header")?;
        let kind = header[0];
        let len = header[1] as usize;
        if len < 4 {
            return Err("SMBIOS structure length below header size".into());
        }
        let record = data
            .get(pos..pos + len)
            .ok_or("Truncated SMBIOS formatted record")?;
        let tail = data
            .get(pos + len..)
            .ok_or("Missing SMBIOS string section")?;
        let end = tail
            .windows(2)
            .position(|w| w == [0, 0])
            .ok_or("Unterminated SMBIOS strings")?;
        string_bytes += end;
        if string_bytes > MAX_STRING_BYTES {
            return Err("SMBIOS string data exceeds safety bound".into());
        }
        // String indices are u8. A malformed section must not allocate an
        // unbounded vector of references or a huge UI label from firmware.
        let strings = tail[..end].split(|b| *b == 0).take(256).collect::<Vec<_>>();
        if strings.len() > 255 || strings.iter().any(|s| s.len() > 4096) {
            return Err("SMBIOS string count or length exceeds safety bound".into());
        }
        let add_string = |fields: &mut Vec<Field>, label: &str, offset: usize| {
            if let Some(value) = string_at(record, offset, &strings) {
                if firmware_placeholder(&value) {
                    fields.push(Field::missing(
                        label,
                        Availability::Unavailable,
                        &source,
                        format!("Firmware reports {value}"),
                    ));
                } else {
                    fields.push(Field::valid(label, value, "", &source));
                }
            }
        };
        match kind {
            0 => {
                add_string(&mut result.motherboard, "BIOS vendor", 4);
                add_string(&mut result.motherboard, "BIOS version", 5);
                add_string(&mut result.motherboard, "BIOS date", 8);
            }
            2 => {
                add_string(&mut result.motherboard, "Manufacturer", 4);
                add_string(&mut result.motherboard, "Model", 5);
                add_string(&mut result.motherboard, "Revision", 6);
            }
            4 => {
                add_string(&mut result.cpu, "Socket", 4);
                if let Some(speed) = u16_at(record, 22).filter(|v| *v != 0) {
                    result.cpu.push(Field::valid(
                        "Firmware configured frequency",
                        speed.to_string(),
                        "MHz",
                        &source,
                    ));
                }
            }
            17 => {
                let size = u16_at(record, 12);
                if size != Some(0) {
                    module += 1;
                    let prefix = format!("Module {module}");
                    for (label, offset) in [("locator", 16), ("bank", 17), ("part", 26)] {
                        add_string(&mut result.memory, &format!("{prefix} {label}"), offset);
                    }
                    result.memory.push(module_manufacturer(
                        record,
                        &strings,
                        &format!("{prefix} manufacturer"),
                        &source,
                    ));
                    match module_capacity(record) {
                        Some(capacity) => {
                            installed = installed.saturating_add(capacity);
                            result.memory.push(Field::valid(
                                format!("{prefix} capacity"),
                                bytes(capacity),
                                "",
                                &source,
                            ));
                        }
                        None => {
                            unknown_capacity = true;
                            result.memory.push(Field::missing(
                                format!("{prefix} capacity"),
                                Availability::Unavailable,
                                &source,
                                "Firmware omitted module capacity",
                            ));
                        }
                    }
                    if let Some(ty) = record.get(18) {
                        result.memory.push(Field::valid(
                            format!("{prefix} type"),
                            memory_type(*ty),
                            "",
                            &source,
                        ));
                    }
                    for (label, offset, extended) in [
                        ("rated transfer rate", 21, 84),
                        ("configured transfer rate", 32, 88),
                    ] {
                        if let Some(speed) = transfer_rate(record, offset, extended) {
                            result.memory.push(Field::valid(
                                format!("{prefix} {label}"),
                                speed.to_string(),
                                "MT/s",
                                &source,
                            ));
                        } else {
                            result.memory.push(Field::missing(
                                format!("{prefix} {label}"),
                                Availability::Unavailable,
                                &source,
                                "Firmware omitted the transfer rate",
                            ));
                        }
                    }
                    if let Some(width) = u16_at(record, 10).filter(|v| *v != 0 && *v != u16::MAX) {
                        result.memory.push(Field::valid(
                            format!("{prefix} data width"),
                            width.to_string(),
                            "bits",
                            &source,
                        ));
                    }
                }
            }
            _ => {}
        }
        pos += len + end + 2;
        if kind == 127 {
            break;
        }
    }
    if installed > 0 && !unknown_capacity {
        result.memory.insert(
            0,
            Field::valid("Installed capacity", bytes(installed), "", source),
        );
    } else {
        result.memory.insert(
            0,
            Field::missing(
                "Installed capacity",
                Availability::Unavailable,
                source,
                "Complete DIMM capacities not supplied by firmware",
            ),
        );
    }
    Ok(())
}

fn module_capacity(record: &[u8]) -> Option<u64> {
    let size = u16_at(record, 12)?;
    match size {
        0xffff => None,
        0x7fff => u32_at(record, 28)
            .map(|v| ((v & 0x7fff_ffff) as u64) * 1024 * 1024)
            .filter(|v| *v > 0),
        value if value & 0x8000 != 0 => Some((value & 0x7fff) as u64 * 1024),
        value => Some(value as u64 * 1024 * 1024),
    }
}
fn transfer_rate(record: &[u8], offset: usize, extended: usize) -> Option<u32> {
    match u16_at(record, offset)? {
        0 => None,
        0xffff => u32_at(record, extended).filter(|v| *v != 0),
        v => Some(v as u32),
    }
}
fn memory_type(value: u8) -> String {
    match value {
        18 => "DDR",
        19 => "DDR2",
        24 => "DDR3",
        26 => "DDR4",
        27 => "LPDDR",
        28 => "LPDDR2",
        29 => "LPDDR3",
        30 => "LPDDR4",
        34 => "DDR5",
        35 => "LPDDR5",
        _ => return format!("SMBIOS type {value}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw_table(data: &[u8]) -> Vec<u8> {
        let mut raw = vec![0, 3, 6, 0];
        raw.extend_from_slice(&(data.len() as u32).to_le_bytes());
        raw.extend_from_slice(data);
        raw
    }

    #[test]
    fn malformed_or_expanding_firmware_never_publishes_partial_inventory() {
        let mut inv = Inventory::default();
        inv.cpu
            .push(Field::valid("Existing", "retained", "", "test"));
        let mut record = vec![2, 8, 0, 0, 1, 0, 0, 0];
        record.extend_from_slice(b"Example board\0\0");
        record.push(17); // Valid identification followed by a truncated header.
        assert!(parse(&raw_table(&record), &mut inv).is_err());
        assert!(inv.motherboard.is_empty());
        assert!(inv.memory.is_empty());
        assert_eq!(inv.cpu.len(), 1);

        let repeated = [2, 4, 0, 0, 0, 0].repeat(MAX_RECORDS + 1);
        assert!(parse(&raw_table(&repeated), &mut inv).is_err());
        let mut oversized = vec![2, 8, 0, 0, 1, 0, 0, 0];
        oversized.extend(std::iter::repeat_n(b'x', 4097));
        oversized.extend_from_slice(&[0, 0]);
        assert!(parse(&raw_table(&oversized), &mut inv).is_err());
        assert!(inv.motherboard.is_empty());
    }
    #[test]
    fn memory_size_sentinels_and_transfer_rate_units() {
        let mut record = vec![0u8; 92];
        record[12..14].copy_from_slice(&0xffffu16.to_le_bytes());
        assert_eq!(module_capacity(&record), None);
        record[12..14].copy_from_slice(&0x7fffu16.to_le_bytes());
        record[28..32].copy_from_slice(&65536u32.to_le_bytes());
        assert_eq!(module_capacity(&record), Some(64 * 1024 * 1024 * 1024));
        record[12..14].copy_from_slice(&0x8400u16.to_le_bytes());
        assert_eq!(module_capacity(&record), Some(1024 * 1024));
        record[32..34].copy_from_slice(&6000u16.to_le_bytes());
        assert_eq!(transfer_rate(&record, 32, 88), Some(6000));
        record[32..34].copy_from_slice(&0xffffu16.to_le_bytes());
        record[88..92].copy_from_slice(&12800u32.to_le_bytes());
        assert_eq!(transfer_rate(&record, 32, 88), Some(12800));
    }
    #[test]
    fn malformed_tables_cannot_overrun_or_loop() {
        for raw in [
            &[][..],
            &[0; 7][..],
            &[0, 3, 6, 0, 255, 255, 255, 255][..],
            &[0, 3, 6, 0, 4, 0, 0, 0, 17, 0, 0, 0][..],
            &[0, 3, 6, 0, 4, 0, 0, 0, 17, 4, 0, 0][..],
        ] {
            assert!(parse(raw, &mut Inventory::default()).is_err());
        }
    }
    #[test]
    fn module_manufacturer_code_recovers_missing_name_without_guessing() {
        for (name, id, expected) in [
            ("Unknown", [0x04, 0xef], Some("Team Group Inc.")),
            ("", [0x04, 0xef], Some("Team Group Inc.")),
            ("Named vendor", [0x04, 0xef], Some("Named vendor")),
            ("Unknown", [0, 0], None),
            ("Unknown", [0xef, 0x04], None),
            ("Unknown", [0x04, 0x6f], None),
        ] {
            let mut record = vec![0; 46];
            record[0] = 17;
            record[1] = 46;
            record[12..14].copy_from_slice(&16384u16.to_le_bytes());
            record[23] = 1;
            record[44..46].copy_from_slice(&id);
            record.extend_from_slice(name.as_bytes());
            record.extend_from_slice(&[0, 0]);
            let mut inv = Inventory::default();
            parse(&raw_table(&record), &mut inv).unwrap();
            let manufacturer = inv
                .memory
                .iter()
                .find(|f| f.label == "Module 1 manufacturer")
                .unwrap();
            assert_eq!(manufacturer.value.as_deref(), expected, "{name}, {id:02x?}");
        }
        let mut board = vec![2, 8, 0, 0, 1, 2, 3, 0];
        board.extend_from_slice(b"Gigabyte\0X870 EAGLE WIFI7\0x.x\0\0");
        let mut inv = Inventory::default();
        parse(&raw_table(&board), &mut inv).unwrap();
        let revision = inv
            .motherboard
            .iter()
            .find(|f| f.label == "Revision")
            .unwrap();
        assert!(revision.value.is_none());
        assert!(revision.detail.contains("x.x"));
    }
    #[test]
    fn firmware_strings_and_unknown_capacity_remain_explicit() {
        let mut record = vec![0u8; 34];
        record[0] = 17;
        record[1] = 34;
        record[16] = 1;
        record[23] = 2;
        record[12..14].copy_from_slice(&0xffffu16.to_le_bytes());
        record.extend_from_slice(b"DIMM_A2\0Unknown\0\0");
        let mut raw = vec![0, 3, 6, 0];
        raw.extend_from_slice(&(record.len() as u32).to_le_bytes());
        raw.extend(record);
        let mut inv = Inventory::default();
        parse(&raw, &mut inv).unwrap();
        assert!(
            inv.memory
                .iter()
                .any(|f| f.label == "Module 1 locator" && f.value.as_deref() == Some("DIMM_A2"))
        );
        assert!(
            inv.memory
                .iter()
                .find(|f| f.label == "Installed capacity")
                .unwrap()
                .value
                .is_none()
        );
        let manufacturer = inv
            .memory
            .iter()
            .find(|f| f.label == "Module 1 manufacturer")
            .unwrap();
        assert_eq!(manufacturer.state, Availability::Unavailable);
        assert!(manufacturer.value.is_none());
    }
}
