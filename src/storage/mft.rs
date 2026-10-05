//! Optional whole-volume NTFS index. USN records provide hierarchy only; sizes
//! still come from the same handle-based stream accounting as normal scans.
//! Additional hard-link names are enumerated explicitly. No raw NTFS parsing,
//! journal mutation, privilege adjustment, or automatic elevation is used.

use super::{Worker, native};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    mem::size_of,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FindClose, FindFirstFileNameW, FindNextFileNameW, GetFileInformationByHandle,
            OPEN_EXISTING,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{FSCTL_ENUM_USN_DATA, MFT_ENUM_DATA_V0},
        },
    },
    core::{PCWSTR, PWSTR},
};

struct Record {
    reference: u64,
    parent: u64,
    attributes: u32,
    name: OsString,
}

struct Index {
    root_reference: u64,
    children: HashMap<u64, Vec<Record>>,
}

const MAX_INDEX_BYTES: usize = 128 * 1024 * 1024;
const MAX_LINK_NAMES: usize = 4096;
const MAX_LINK_NAME_BYTES: usize = 32 * 1024 * 1024;

/// Returns true only when this path was handled (including cancellation). A
/// failed preflight/index falls back before emitting nodes, so IDs remain clean.
pub(super) fn scan_volume(root: &Path, worker: &mut Worker) -> bool {
    let Some(letter) = drive_root(root) else {
        return false;
    };
    let index = match read_index(root, letter, worker) {
        Ok(Some(index)) => index,
        Ok(None) => return true,
        Err(error) => {
            worker.summary.notes.push(format!("MFT fast path unavailable: {error}. Standard enumeration used; no elevation requested."));
            return false;
        }
    };
    worker.summary.method = "NTFS MFT index + verified stream metadata".into();
    worker.summary.notes.push("MFT index is a live hierarchy, not a snapshot. Hard-link names are reconciled explicitly. NTFS reserved metadata records are excluded; inaccessible and changing entries remain partial.".into());
    let mut children = index.children;
    let Some(visited) = worker.visit(root.to_path_buf(), None) else {
        return true;
    };
    if !visited.traverse {
        return true;
    }
    let mut directory_ids = HashMap::new();
    directory_ids.insert(index.root_reference, (visited.id, root.to_path_buf()));
    let mut stack = vec![index.root_reference];
    let mut files = Vec::new();
    let mut retained_paths = 0usize;
    let mut excluded_directories = Vec::new();
    let mut visited_directories = HashSet::new();
    // Directories arrive before any file. This permits an extra hard-link name
    // to be attached to its actual parent even if it was absent from the index.
    while let Some(reference) = stack.pop() {
        if worker.cancelled() {
            return true;
        }
        if !visited_directories.insert(reference) {
            worker.record_error(root, "Cycle in MFT directory references; branch excluded");
            continue;
        }
        let (parent_id, parent_path) = directory_ids.get(&reference).cloned().unwrap();
        for record in children.remove(&reference).unwrap_or_default() {
            if worker.cancelled() {
                return true;
            }
            let path = parent_path.join(&record.name);
            retained_paths =
                retained_paths.saturating_add(path.capacity() + size_of::<PathBuf>() + 96);
            if retained_paths > MAX_INDEX_BYTES {
                worker.record_error(root, "MFT hierarchy paths reached the 128 MiB memory budget; remaining entries were not scanned. Scan a smaller folder to continue.");
                worker.stopped = true;
                return true;
            }
            if record.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
                if let Some(visited) = worker.visit(path.clone(), Some(parent_id)) {
                    if visited.traverse {
                        directory_ids.insert(record.reference, (visited.id, path));
                        stack.push(record.reference);
                    } else if visited.excluded {
                        excluded_directories.push(record.reference);
                    }
                }
            } else {
                files.push((parent_id, path));
            }
            if !worker.flush(false) {
                return true;
            }
        }
    }
    // Reparse/cloud directory descendants never enter directory_ids or files.
    // A live index can also contain orphaned references after a rename/delete.
    // Expose the omitted record count instead of silently calling that complete.
    let excluded = prune_branches(&mut children, excluded_directories);
    if excluded > 0 {
        worker.summary.notes.push(format!(
            "MFT index excluded {excluded} descendant records beneath reparse/cloud directories."
        ));
    }
    let omitted = children.values().map(Vec::len).sum::<usize>();
    if omitted > 0 {
        worker.record_error(root, &format!("MFT hierarchy has {omitted} unreachable records beneath inaccessible or missing directory references; totals cover reachable entries only"));
    }
    // Free orphan records before the expensive metadata pass.
    drop(children);
    let mut emitted = HashSet::new();
    for (parent, path) in files {
        if worker.cancelled() {
            return true;
        }
        if emitted.contains(&path) {
            continue;
        }
        if let Some(visited) = worker.visit(path.clone(), Some(parent))
            && visited.links > 1
        {
            // Single-link paths do not need retention in the deduplication set.
            emitted.insert(path.clone());
            match link_names(&path) {
                Ok(names) => {
                    for relative in names {
                        if worker.cancelled() {
                            return true;
                        }
                        let alias = root.join(relative);
                        if emitted.contains(&alias) {
                            continue;
                        }
                        let Some(alias_parent) = alias.parent() else {
                            continue;
                        };
                        match file_reference(alias_parent) {
                            Ok(reference) => {
                                if let Some((parent_id, _)) = directory_ids.get(&reference) {
                                    emitted.insert(alias.clone());
                                    worker.visit(alias, Some(*parent_id));
                                    if !worker.flush(false) {
                                        return true;
                                    }
                                }
                            }
                            Err(error) => worker.record_error(
                                &alias,
                                &format!("Hard-link parent unavailable: {error}"),
                            ),
                        }
                    }
                }
                Err(error) => worker.record_error(
                    &path,
                    &format!("Additional hard-link names unavailable: {error}"),
                ),
            }
        }
        if !worker.flush(false) {
            return true;
        }
    }
    true
}

fn drive_root(path: &Path) -> Option<char> {
    let name = path.to_string_lossy();
    let name = name.strip_prefix(r"\\?\").unwrap_or(&name);
    let chars: Vec<char> = name.chars().collect();
    (chars.len() == 3
        && chars[1] == ':'
        && matches!(chars[2], '\\' | '/')
        && chars[0].is_ascii_alphabetic())
    .then(|| chars[0])
}

fn file_reference(path: &Path) -> Result<u64, String> {
    let handle = native::metadata_handle(path).map_err(|e| e.to_string())?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle.0, &mut info) }.map_err(|e| e.to_string())?;
    if native::skip_attributes(info.dwFileAttributes).is_some() {
        return Err("Reparse/cloud entry excluded".into());
    }
    Ok(((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64)
}

fn read_index(root: &Path, letter: char, worker: &Worker) -> Result<Option<Index>, String> {
    let volume: Vec<u16> = format!(r"\\.\{letter}:")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(volume.as_ptr()),
            0x80000000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map(native::OwnedHandle)
    .map_err(|e| format!("volume read handle: {e}"))?;
    let root_reference = file_reference(root)?;
    let mut request = MFT_ENUM_DATA_V0 {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: i64::MAX,
    };
    let mut words = vec![0u64; 131_072]; // 1 MiB, naturally aligned.
    let mut children: HashMap<u64, Vec<Record>> = HashMap::new();
    let mut retained_bytes = 0usize;
    loop {
        if worker.cancelled() {
            return Ok(None);
        }
        let mut returned = 0;
        let result = unsafe {
            DeviceIoControl(
                handle.0,
                FSCTL_ENUM_USN_DATA,
                Some((&request as *const MFT_ENUM_DATA_V0).cast()),
                size_of::<MFT_ENUM_DATA_V0>() as u32,
                Some(words.as_mut_ptr().cast()),
                (words.len() * 8) as u32,
                Some(&mut returned),
                None,
            )
        };
        if let Err(error) = result {
            if error.code().0 as u32 & 0xffff == 38 {
                break;
            } // ERROR_HANDLE_EOF
            return Err(format!("FSCTL_ENUM_USN_DATA: {error}"));
        }
        if returned < 8 || returned as usize > words.len() * 8 {
            return Err("invalid MFT response length".into());
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), returned as usize) };
        let next = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        if next <= request.StartFileReferenceNumber {
            return Err("MFT cursor did not advance".into());
        }
        let records = parse_records(&bytes[8..])?;
        for record in records {
            // NTFS reserves records 0..23 for filesystem structures. These are
            // deliberately excluded from file totals, including their children.
            if record.reference == root_reference || record.reference & 0x0000_ffff_ffff_ffff < 24 {
                continue;
            }
            // Include record/name ownership plus conservative hash/vector
            // overhead. Oversized indices fall back before emitting any nodes.
            retained_bytes =
                retained_bytes.saturating_add(size_of::<Record>() + record.name.len() + 96);
            if retained_bytes > MAX_INDEX_BYTES {
                return Err("MFT index exceeded its 128 MiB memory budget".into());
            }
            children.entry(record.parent).or_default().push(record);
        }
        request.StartFileReferenceNumber = next;
    }
    let reserved = children
        .keys()
        .copied()
        .filter(|reference| *reference != root_reference && reference & 0x0000_ffff_ffff_ffff < 24)
        .collect();
    prune_branches(&mut children, reserved);
    Ok(Some(Index {
        root_reference,
        children,
    }))
}

fn prune_branches(children: &mut HashMap<u64, Vec<Record>>, mut references: Vec<u64>) -> usize {
    let mut removed = 0;
    while let Some(reference) = references.pop() {
        for record in children.remove(&reference).unwrap_or_default() {
            removed += 1;
            references.push(record.reference);
        }
    }
    removed
}

fn parse_records(bytes: &[u8]) -> Result<Vec<Record>, String> {
    let mut offset = 0usize;
    let mut records = Vec::new();
    while offset < bytes.len() {
        let record = &bytes[offset..];
        if record.len() < 60 {
            return Err("truncated USN record".into());
        }
        let length = u32::from_le_bytes(record[..4].try_into().unwrap()) as usize;
        let major = u16::from_le_bytes(record[4..6].try_into().unwrap());
        if major != 2 {
            return Err(format!("USN record version {major} is unsupported"));
        }
        if length < 60 || length > record.len() {
            return Err("invalid USN record length".into());
        }
        let name_length = u16::from_le_bytes(record[56..58].try_into().unwrap()) as usize;
        let name_offset = u16::from_le_bytes(record[58..60].try_into().unwrap()) as usize;
        if name_offset < 60 || !name_length.is_multiple_of(2) || name_offset + name_length > length
        {
            return Err("invalid USN name bounds".into());
        }
        let name: Vec<u16> = record[name_offset..name_offset + name_length]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if name.is_empty()
            || name.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
            || name == [46]
            || name == [46, 46]
        {
            return Err("invalid USN path component".into());
        }
        records.push(Record {
            reference: u64::from_le_bytes(record[8..16].try_into().unwrap()),
            parent: u64::from_le_bytes(record[16..24].try_into().unwrap()),
            attributes: u32::from_le_bytes(record[52..56].try_into().unwrap()),
            name: OsString::from_wide(&name),
        });
        offset += length;
    }
    Ok(records)
}

fn link_names(path: &Path) -> Result<Vec<PathBuf>, String> {
    let name = native::wide(path);
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    let handle = unsafe {
        FindFirstFileNameW(
            PCWSTR(name.as_ptr()),
            0,
            &mut length,
            PWSTR(buffer.as_mut_ptr()),
        )
    }
    .map_err(|e| e.to_string())?;
    struct FindHandle(windows::Win32::Foundation::HANDLE);
    impl Drop for FindHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = FindClose(self.0);
            }
        }
    }
    let handle = FindHandle(handle);
    let mut paths = Vec::new();
    let mut retained_bytes = 0usize;
    loop {
        let end = buffer
            .iter()
            .position(|c| *c == 0)
            .ok_or("unterminated hard-link name")?;
        let relative = &buffer[..end];
        if relative.first() != Some(&92) {
            return Err("hard-link name is not volume-relative".into());
        }
        let path = PathBuf::from(OsString::from_wide(&relative[1..]));
        if path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("invalid hard-link path".into());
        }
        retain_link_name(&mut paths, &mut retained_bytes, path)?;
        length = buffer.len() as u32;
        match unsafe { FindNextFileNameW(handle.0, &mut length, PWSTR(buffer.as_mut_ptr())) } {
            Ok(()) => {}
            Err(error) if error.code().0 as u32 & 0xffff == 38 => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(paths)
}

fn retain_link_name(
    paths: &mut Vec<PathBuf>,
    retained_bytes: &mut usize,
    path: PathBuf,
) -> Result<(), String> {
    let next = retained_bytes.saturating_add(path.capacity() + size_of::<PathBuf>() * 2);
    if paths.len() >= MAX_LINK_NAMES || next > MAX_LINK_NAME_BYTES {
        return Err("Hard-link name enumeration exceeded its count or memory budget; additional paths are unavailable".into());
    }
    paths.push(path);
    *retained_bytes = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn record(name: &str, reference: u64, parent: u64) -> Vec<u8> {
        let name: Vec<u16> = name.encode_utf16().collect();
        let length = (60 + name.len() * 2 + 7) & !7;
        let mut bytes = vec![0u8; length];
        bytes[..4].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        bytes[8..16].copy_from_slice(&reference.to_le_bytes());
        bytes[16..24].copy_from_slice(&parent.to_le_bytes());
        bytes[56..58].copy_from_slice(&((name.len() * 2) as u16).to_le_bytes());
        bytes[58..60].copy_from_slice(&60u16.to_le_bytes());
        for (index, value) in name.iter().enumerate() {
            bytes[60 + index * 2..62 + index * 2].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn usn_parser_preserves_unicode_and_rejects_unknown_versions_and_traversal() {
        let bytes = record("Grüße_日本語.bin", 44, 5);
        let parsed = parse_records(&bytes).unwrap();
        assert_eq!(parsed[0].name, OsString::from("Grüße_日本語.bin"));
        assert_eq!(parsed[0].reference, 44);
        assert_eq!(parsed[0].parent, 5);
        assert!(parse_records(&bytes[..bytes.len() - 2]).is_err());
        let mut unknown = bytes.clone();
        unknown[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert!(parse_records(&unknown).is_err());
        assert!(parse_records(&record("..", 44, 5)).is_err());
        assert!(parse_records(&record(".", 44, 5)).is_err());
        assert!(parse_records(&record(r"outside\file", 44, 5)).is_err());
    }

    #[test]
    fn mft_hardlink_enumeration_returns_all_names() {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("original.bin");
        fs::write(&original, b"data").unwrap();
        fs::hard_link(&original, fixture.path().join("alias.bin")).unwrap();
        let names = link_names(&original).unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.ends_with("alias.bin")));
        assert!(names.iter().any(|n| n.ends_with("original.bin")));
        let mut retained = 0;
        let mut limited = Vec::new();
        for _ in 0..MAX_LINK_NAMES {
            retain_link_name(&mut limited, &mut retained, PathBuf::from("file.bin")).unwrap();
        }
        assert!(
            retain_link_name(&mut limited, &mut retained, PathBuf::from("overflow.bin")).is_err()
        );
        assert_eq!(limited.len(), MAX_LINK_NAMES);
        limited.clear();
        retained = MAX_LINK_NAME_BYTES;
        assert!(
            retain_link_name(&mut limited, &mut retained, PathBuf::from("日本語.bin")).is_err()
        );
        assert!(
            limited.is_empty(),
            "an exhausted budget must not retain another path"
        );
    }

    #[test]
    fn only_drive_roots_attempt_mft() {
        assert_eq!(drive_root(Path::new(r"C:\")), Some('C'));
        assert_eq!(drive_root(Path::new(r"\\?\D:\")), Some('D'));
        assert_eq!(drive_root(Path::new(r"C:\Users")), None);
        assert_eq!(drive_root(Path::new(r"\\server\share\")), None);
    }

    #[test]
    fn excluded_mft_branches_remove_all_descendants_only() {
        let mut children: HashMap<u64, Vec<Record>> = HashMap::new();
        for (name, reference, parent) in [
            ("user.bin", 44, 5),
            ("metadata", 30, 11),
            ("nested", 31, 30),
        ] {
            let parsed = parse_records(&record(name, reference, parent))
                .unwrap()
                .pop()
                .unwrap();
            children.entry(parent).or_default().push(parsed);
        }
        assert_eq!(prune_branches(&mut children, vec![11]), 2);
        assert_eq!(children.len(), 1);
        assert_eq!(children[&5][0].name, OsString::from("user.bin"));
    }
}
