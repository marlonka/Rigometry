//! The Windows boundary reads metadata through handles opened with no data
//! access and no recall. Every handle is owned and closed on every exit path.

use super::{EntryInfo, FileIdentity, Refusal, Skip, append_note};
use std::{fs, mem::size_of, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_COMPRESSED,
            FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_ATTRIBUTE_SPARSE_FILE, FILE_COMPRESSION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO,
            FileCompressionInfo, FileIdInfo, FileStandardInfo, FileStreamInfo,
            GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
        },
    },
    core::PCWSTR,
};

pub(super) struct OwnedHandle(pub(super) HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(super) fn wide(path: &Path) -> Vec<u16> {
    // std::fs accepts long paths, but direct Win32 calls need the extended-path
    // prefix independently of the executable's longPathAware manifest.
    let path = std::path::absolute(path).unwrap_or_else(|_| path.into());
    let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut value = if raw.starts_with(&[92, 92, 63, 92]) {
        raw
    } else if raw.starts_with(&[92, 92]) {
        let mut out: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        out.extend_from_slice(&raw[2..]);
        out
    } else {
        let mut out: Vec<u16> = r"\\?\".encode_utf16().collect();
        out.extend(raw);
        out
    };
    value.push(0);
    value
}

/// User paths must name filesystem entries, never DOS devices, named pipes,
/// physical disks, arbitrary NT namespaces or alternate streams to be modified.
/// Internally enumerated ADS still use metadata_handle directly.
pub(super) fn validate_path(path: &Path) -> Result<(), String> {
    use std::path::{Component, Prefix};
    if path.as_os_str().is_empty() || path.as_os_str().encode_wide().any(|c| c == 0) {
        return Err("Path is empty or contains a NUL character".into());
    }
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(_)
                | Prefix::VerbatimDisk(_)
                | Prefix::UNC(_, _)
                | Prefix::VerbatimUNC(_, _) => {}
                _ => return Err("Device and NT namespace paths are not supported".into()),
            },
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.contains([':', '*', '?', '"', '<', '>', '|']) {
                    return Err(
                        "Alternate streams and non-filesystem path syntax are not supported".into(),
                    );
                }
                let base = name
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(' ')
                    .to_ascii_uppercase();
                let numbered_device = ["COM", "LPT"].iter().any(|prefix| {
                    base.strip_prefix(prefix).is_some_and(|suffix| {
                        matches!(
                            suffix,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                });
                if matches!(
                    base.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                ) || numbered_device
                {
                    return Err("Reserved DOS device names are not supported".into());
                }
            }
            Component::ParentDir if path.as_os_str().encode_wide().take(4).eq([92, 92, 63, 92]) => {
                return Err("Parent traversal in an extended path is not supported".into());
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn metadata_handle(path: &Path) -> windows::core::Result<OwnedHandle> {
    let name = wide(path);
    // Desired access = 0: query-only handle. This avoids requiring data-read
    // permission and cannot request cloud content. Reparse processing disabled.
    unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
            None,
        )
        .map(OwnedHandle)
    }
}

/// The Win32 error inside an HRESULT_FROM_WIN32 value (facility 7).
pub(super) fn win32_code(error: &windows::core::Error) -> Option<u32> {
    let code = error.code().0 as u32;
    (code >> 16 == 0x8007).then_some(code & 0xffff)
}

/// A Windows error as `std::io` reports it: Win32 errors keep their code,
/// so access-denied and sharing violations are recognized as refusals.
pub(super) fn io_error(error: windows::core::Error) -> std::io::Error {
    match win32_code(&error) {
        Some(code) => std::io::Error::from_raw_os_error(code as i32),
        None => error.into(),
    }
}

pub(super) fn skip_attributes(attributes: u32) -> Option<Skip> {
    let cloud = FILE_ATTRIBUTE_OFFLINE.0
        | FILE_ATTRIBUTE_RECALL_ON_OPEN.0
        | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0;
    if attributes & cloud != 0 {
        Some(Skip::Cloud)
    } else if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        Some(Skip::Reparse)
    } else {
        None
    }
}

pub(super) fn query<T: Default>(
    handle: HANDLE,
    class: windows::Win32::Storage::FileSystem::FILE_INFO_BY_HANDLE_CLASS,
) -> windows::core::Result<T> {
    let mut result = T::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            class,
            (&mut result as *mut T).cast(),
            size_of::<T>() as u32,
        )?;
    }
    Ok(result)
}

pub(super) fn read_entry(path: &Path, metadata: &fs::Metadata) -> EntryInfo {
    let mut result = EntryInfo {
        logical: if metadata.is_dir() { 0 } else { metadata.len() },
        ..Default::default()
    };
    let handle = match metadata_handle(path) {
        Ok(handle) => handle,
        Err(error) => {
            result.errors = 1;
            result.refusal = Refusal::of_os_error(win32_code(&error).map(|code| code as i32));
            result.note = format!("Allocation and stream metadata unavailable: {error}");
            result.allocated = locked_allocation_lower_bound(metadata);
            return result;
        }
    };
    let mut basic = BY_HANDLE_FILE_INFORMATION::default();
    if let Err(error) = unsafe { GetFileInformationByHandle(handle.0, &mut basic) } {
        result.errors = 1;
        result.note = format!("Metadata unavailable: {error}");
        return result;
    }
    // Check the object actually opened, not only potentially outdated directory
    // metadata. A replaced file must not become a followed reparse target.
    if let Some(skip) = skip_attributes(basic.dwFileAttributes) {
        result.skip = Some(skip);
        return result;
    }
    result.links = basic.nNumberOfLinks as u64;
    // Link counts can change between visited paths. Retain identity even when
    // this particular handle currently reports one link, so 1→2 and 2→1
    // transitions cannot count the same physical stream allocation twice.
    result.identity = match query::<FILE_ID_INFO>(handle.0, FileIdInfo) {
        Ok(id) if id.FileId.Identifier != [0; 16] => {
            Some(FileIdentity(id.VolumeSerialNumber, id.FileId.Identifier))
        }
        _ => {
            let mut id = [0; 16];
            id[..8].copy_from_slice(
                &(((basic.nFileIndexHigh as u64) << 32) | basic.nFileIndexLow as u64).to_le_bytes(),
            );
            // Some network filesystems report all-zero IDs. Never deduplicate
            // these unrelated files as though zero were a valid identity.
            if id == [0; 16] {
                if result.links > 1 {
                    result.errors += 1;
                    append_note(
                        &mut result.note,
                        "Stable file identity unavailable; physical allocation excluded to avoid double-counting",
                    );
                }
                None
            } else {
                Some(FileIdentity(basic.dwVolumeSerialNumber as u64, id))
            }
        }
    };
    // FileStreamInfo enumerates metadata only, including directory ADS, and
    // reports each stream's separate logical and allocated sizes.
    match streams(handle.0) {
        Ok(streams) => {
            accumulate_stream_sizes(&mut result, &streams);
            let named = streams.iter().filter(|s| !s.is_default()).count();
            if named > 0 {
                append_note(
                    &mut result.note,
                    &format!("Includes {named} named data stream(s)"),
                );
            }
            // For sparse/compressed streams the reserved allocation may exceed
            // committed bytes. Query compression information per stream handle.
            for stream in &streams {
                let is_default = stream.is_default();
                let stream_handle;
                let stream_raw = if is_default {
                    handle.0
                } else {
                    let mut stream_path = path.as_os_str().to_os_string();
                    use std::os::windows::ffi::OsStringExt;
                    stream_path.push(std::ffi::OsString::from_wide(&stream.name));
                    stream_handle = match metadata_handle(Path::new(&stream_path)) {
                        Ok(handle) => handle,
                        Err(error) => {
                            result.errors += 1;
                            result.allocated = result.allocated.saturating_sub(stream.allocated);
                            append_note(
                                &mut result.note,
                                &format!("Named stream allocation unverified: {error}"),
                            );
                            continue;
                        }
                    };
                    stream_handle.0
                };
                let mut attributes = basic.dwFileAttributes;
                if !is_default {
                    let mut stream_info = BY_HANDLE_FILE_INFORMATION::default();
                    match unsafe { GetFileInformationByHandle(stream_raw, &mut stream_info) } {
                        Ok(()) => attributes = stream_info.dwFileAttributes,
                        Err(error) => {
                            result.errors += 1;
                            result.allocated = result.allocated.saturating_sub(stream.allocated);
                            append_note(
                                &mut result.note,
                                &format!("Named stream allocation unavailable: {error}"),
                            );
                            continue;
                        }
                    }
                }
                if attributes & (FILE_ATTRIBUTE_SPARSE_FILE.0 | FILE_ATTRIBUTE_COMPRESSED.0) != 0 {
                    match query::<FILE_COMPRESSION_INFO>(stream_raw, FileCompressionInfo) {
                        Ok(compression) => {
                            replace_stream_allocation(
                                &mut result,
                                stream.allocated,
                                compression.CompressedFileSize,
                            );
                        }
                        Err(error) => {
                            result.errors += 1;
                            result.allocated = result.allocated.saturating_sub(stream.allocated);
                            append_note(
                                &mut result.note,
                                &format!(
                                    "Sparse/compressed physical allocation unavailable: {error}"
                                ),
                            );
                        }
                    }
                }
            }
        }
        Err(error) => {
            // Only query standard information when stream enumeration failed;
            // a successful stream response already includes the default stream.
            match query::<FILE_STANDARD_INFO>(handle.0, FileStandardInfo) {
                Ok(info) if !info.Directory => {
                    result.logical = info.EndOfFile.max(0) as u64;
                    result.allocated = info.AllocationSize.max(0) as u64;
                    if basic.dwFileAttributes
                        & (FILE_ATTRIBUTE_SPARSE_FILE.0 | FILE_ATTRIBUTE_COMPRESSED.0)
                        != 0
                    {
                        match query::<FILE_COMPRESSION_INFO>(handle.0, FileCompressionInfo) {
                            Ok(compression) => {
                                let previous = result.allocated;
                                replace_stream_allocation(
                                    &mut result,
                                    previous,
                                    compression.CompressedFileSize,
                                );
                            }
                            Err(error) => {
                                result.errors += 1;
                                result.allocated = 0;
                                append_note(
                                    &mut result.note,
                                    &format!("Sparse/compressed allocation unavailable: {error}"),
                                );
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    result.errors += 1;
                    append_note(
                        &mut result.note,
                        &format!("Allocation unavailable: {error}"),
                    );
                }
            }
            // FAT/exFAT do not implement named streams. Preserve the default
            // stream's standard metadata and explain that distinction.
            if matches!(error.code().0 as u32 & 0xffff, 1 | 50 | 87) {
                append_note(
                    &mut result.note,
                    "Named stream enumeration unsupported by this filesystem",
                );
            } else {
                result.errors += 1;
                append_note(
                    &mut result.note,
                    &format!("Named stream metadata unavailable: {error}"),
                );
            }
        }
    }
    if result.links > 1 && result.identity.is_none() {
        result.allocated = 0;
    }
    if basic.dwFileAttributes & FILE_ATTRIBUTE_SPARSE_FILE.0 != 0 {
        append_note(&mut result.note, "Sparse");
    }
    if basic.dwFileAttributes & FILE_ATTRIBUTE_COMPRESSED.0 != 0 {
        append_note(&mut result.note, "NTFS compressed");
    }
    result
}

/// Files Windows holds open exclusively (hiberfil.sys, pagefile.sys) refuse
/// even a query-only handle, but the directory listing still reports their
/// length. Unless sparse or compressed, a file occupies whole clusters for
/// all of its bytes, so its length is a lower bound for its allocation.
/// Small files can live inside their MFT record with no allocation at all.
pub(super) fn locked_allocation_lower_bound(metadata: &fs::Metadata) -> u64 {
    use std::os::windows::fs::MetadataExt;
    const MFT_RESIDENT_LIMIT: u64 = 1024;
    let shrinkable = FILE_ATTRIBUTE_SPARSE_FILE.0 | FILE_ATTRIBUTE_COMPRESSED.0;
    if metadata.is_dir()
        || metadata.file_attributes() & shrinkable != 0
        || metadata.len() <= MFT_RESIDENT_LIMIT
    {
        0
    } else {
        metadata.len()
    }
}

#[derive(Debug)]
pub(super) struct Stream {
    pub logical: u64,
    pub allocated: u64,
    pub name: Vec<u16>,
}

pub(super) fn accumulate_stream_sizes(result: &mut EntryInfo, streams: &[Stream]) {
    for (logical, label) in [
        (true, "Logical stream bytes"),
        (false, "Allocated stream bytes"),
    ] {
        let sum = streams.iter().try_fold(0u64, |sum, stream| {
            sum.checked_add(if logical {
                stream.logical
            } else {
                stream.allocated
            })
        });
        let bytes = sum.unwrap_or_else(|| {
            result.errors += 1;
            append_note(
                &mut result.note,
                &format!("{label} exceed the u64 limit; reported size is a lower bound"),
            );
            u64::MAX
        });
        if logical {
            result.logical = bytes;
        } else {
            result.allocated = bytes;
        }
    }
}

pub(super) fn replace_stream_allocation(result: &mut EntryInfo, reserved: u64, physical: i64) {
    let other_streams = result.allocated.saturating_sub(reserved);
    result.allocated = match u64::try_from(physical) {
        Ok(physical) => other_streams.checked_add(physical).unwrap_or_else(|| {
            result.errors += 1;
            append_note(&mut result.note, "Updated physical stream bytes exceed the u64 limit; reported allocation is a lower bound");
            u64::MAX
        }),
        Err(_) => {
            result.errors += 1;
            append_note(&mut result.note, "Physical allocation query returned a negative byte count; this stream's allocation is unavailable");
            other_streams
        }
    };
}

impl Stream {
    fn is_default(&self) -> bool {
        self.name.is_empty() || self.name == "::$DATA".encode_utf16().collect::<Vec<_>>()
    }
}

fn streams(handle: HANDLE) -> windows::core::Result<Vec<Stream>> {
    let mut words = vec![0u64; 128]; // 8-byte alignment required by Win32.
    loop {
        match unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileStreamInfo,
                words.as_mut_ptr().cast(),
                (words.len() * 8) as u32,
            )
        } {
            Ok(()) => {
                let bytes = unsafe {
                    std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 8)
                };
                return parse_streams(bytes).map_err(|message| {
                    windows::core::Error::new(
                        windows::core::HRESULT(0x8007000d_u32 as i32),
                        message,
                    )
                });
            }
            Err(error) => {
                let code = error.code().0 as u32 & 0xffff;
                if code == 38 {
                    return Ok(Vec::new());
                } // ERROR_HANDLE_EOF: empty directory.
                if matches!(code, 24 | 122 | 234) && words.len() < 131_072 {
                    words.resize(words.len() * 2, 0); // <= 1 MiB per-file bound.
                    continue;
                }
                return Err(error);
            }
        }
    }
}

/// Parse untrusted variable-length driver output without unaligned reads or
/// unchecked offsets. This boundary also handles malformed network responses.
pub(super) fn parse_streams(bytes: &[u8]) -> Result<Vec<Stream>, &'static str> {
    let mut streams = Vec::new();
    let mut offset = 0usize;
    loop {
        let record = bytes.get(offset..).ok_or("Stream offset out of bounds")?;
        if record.len() < 24 {
            return Err("Truncated stream metadata");
        }
        let next = u32::from_le_bytes(record[0..4].try_into().unwrap()) as usize;
        let name_length = u32::from_le_bytes(record[4..8].try_into().unwrap()) as usize;
        let logical = i64::from_le_bytes(record[8..16].try_into().unwrap());
        let allocated = i64::from_le_bytes(record[16..24].try_into().unwrap());
        let end = 24usize
            .checked_add(name_length)
            .ok_or("Stream length overflow")?;
        if logical < 0 || allocated < 0 || !name_length.is_multiple_of(2) || end > record.len() {
            return Err("Invalid stream metadata");
        }
        if next != 0 && (next < end || !next.is_multiple_of(8) || next >= record.len()) {
            return Err("Invalid next stream offset");
        }
        let name: Vec<u16> = record[24..end]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        let suffix: Vec<u16> = ":$DATA".encode_utf16().collect();
        if name.first() != Some(&58)
            || !name.ends_with(&suffix)
            || name.len() < suffix.len() + 1
            || name[1..name.len() - suffix.len()]
                .iter()
                .any(|c| matches!(*c, 0 | 47 | 58 | 92))
        {
            return Err("Invalid stream name");
        }
        streams.push(Stream {
            logical: logical as u64,
            allocated: allocated as u64,
            name,
        });
        if next == 0 {
            return Ok(streams);
        }
        offset = offset.checked_add(next).ok_or("Stream offset overflow")?;
    }
}
