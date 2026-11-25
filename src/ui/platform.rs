//! Operating-system integration for the interface: administrator rights,
//! revealing entries in Explorer or Finder, and platform wording.

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Whether this process already has administrator rights.
#[cfg(windows)]
pub fn elevated() -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ELEVATED.get_or_init(|| unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0;
        let queried = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        let _ = CloseHandle(token);
        queried.is_ok() && elevation.TokenIsElevated != 0
    })
}
/// Starts this executable again with administrator rights, scanning `path`.
/// Windows asks the user for consent; declining returns an error.
#[cfg(windows)]
pub fn relaunch_elevated(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{PCWSTR, w},
    };
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let parameters = wide(scan_argument(path)?.as_ref());
    let executable = wide(executable.as_os_str());
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(executable.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success; lower values are ShellExecute error codes.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err("Administrator scan was not started".into())
    }
}
/// `--scan "path"` for a Windows command line. Paths cannot contain quotes;
/// backslashes before the closing quote are doubled so it stays a quote.
#[cfg(windows)]
pub fn scan_argument(path: &std::path::Path) -> Result<std::ffi::OsString, String> {
    let text = path
        .to_str()
        .ok_or("Path cannot be passed to a new process")?;
    if text.contains(['"', '\0']) {
        return Err("Path cannot be passed to a new process".into());
    }
    let trailing = text.len() - text.trim_end_matches('\\').len();
    Ok(format!("--scan \"{text}{}\"", "\\".repeat(trailing)).into())
}
#[cfg(windows)]
pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    explorer_command(path)?
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
#[cfg(windows)]
pub fn explorer_command(path: &std::path::Path) -> Result<std::process::Command, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    if path
        .as_os_str()
        .encode_wide()
        .any(|unit| unit == 0 || unit == b'"' as u16)
    {
        return Err("Path cannot be passed to Explorer".into());
    }
    let mut directory = vec![0u16; 32768];
    // Obtain the trusted OS location, independent of PATH and the working directory.
    let length = unsafe { GetWindowsDirectoryW(Some(&mut directory)) } as usize;
    if length == 0 || length >= directory.len() {
        return Err("Windows directory is unavailable".into());
    }
    let executable = std::path::PathBuf::from(std::ffi::OsString::from_wide(&directory[..length]))
        .join("explorer.exe");
    let mut argument = std::ffi::OsString::from("/select,\"");
    argument.push(path.as_os_str());
    // A root such as `C:\` ends in a backslash, which would escape the
    // closing quote; doubled, it stays a backslash.
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let trailing = units
        .iter()
        .rev()
        .take_while(|&&unit| unit == u16::from(b'\\'))
        .count();
    argument.push("\\".repeat(trailing));
    argument.push("\"");
    let mut command = std::process::Command::new(executable);
    command.raw_arg(argument);
    Ok(command)
}
/// Administrator scans rely on the Windows consent prompt; other systems never offer them.
#[cfg(not(windows))]
pub fn elevated() -> bool {
    true
}
#[cfg(not(windows))]
pub fn relaunch_elevated(_: &std::path::Path) -> Result<(), String> {
    Err("Administrator scan was not started".into())
}
/// Opens the file manager with `path` selected.
#[cfg(not(windows))]
pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    let mut child = file_manager_command(path)?
        .spawn()
        .map_err(|e| e.to_string())?;
    // Collect the exit status, or every reveal would leave a zombie process.
    std::thread::spawn(move || child.wait());
    Ok(())
}
#[cfg(not(windows))]
pub fn file_manager_command(path: &std::path::Path) -> Result<std::process::Command, String> {
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    // An absolute executable avoids PATH lookup; `--` stops option parsing.
    #[cfg(target_os = "macos")]
    let command = {
        let mut command = std::process::Command::new("/usr/bin/open");
        command.arg("-R").arg("--").arg(&path);
        command
    };
    #[cfg(not(target_os = "macos"))]
    let command = {
        let folder = if path.is_dir() {
            path.clone()
        } else {
            path.parent()
                .map(std::path::PathBuf::from)
                .unwrap_or(path.clone())
        };
        let mut command = std::process::Command::new("/usr/bin/xdg-open");
        command.arg(folder);
        command
    };
    Ok(command)
}
#[cfg(target_os = "macos")]
pub const PLATFORM_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"), "  ·  macOS");
#[cfg(windows)]
pub const PLATFORM_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"), "  ·  Windows x64");
#[cfg(not(any(windows, target_os = "macos")))]
pub const PLATFORM_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"), "  ·  Linux");
/// The key that switches pages: Ctrl on Windows, Command on macOS.
pub const COMMAND_KEY: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl"
};
pub const NO_GPU: &str = if cfg!(windows) {
    "No graphics adapters reported by DXGI"
} else {
    "No graphics adapters reported by the system"
};
pub const REVEAL: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else {
    "Reveal in Explorer"
};
pub const PATH_HINT: &str = if cfg!(windows) {
    "C:\\ or a folder path"
} else {
    "/ or a folder path"
};
