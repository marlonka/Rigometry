use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/rigometry.ico");
    for name in ["RC", "WindowsSdkDir", "WindowsSDKVersion"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if !env::var("TARGET")
        .unwrap_or_default()
        .ends_with("windows-msvc")
    {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/rigometry.ico");
    let rc_file = out.join("rigometry.rc");
    let resource = out.join("rigometry.res");
    let version = env::var("CARGO_PKG_VERSION").expect("package version");
    let numeric_version = ["MAJOR", "MINOR", "PATCH"]
        .map(|part| {
            env::var(format!("CARGO_PKG_VERSION_{part}"))
                .expect("package version component")
                .parse::<u16>()
                .expect("version component fits a Windows version resource")
                .to_string()
        })
        .join(",");
    fs::write(
        &rc_file,
        format!(
            r#"1 ICON "{}"
1 VERSIONINFO
FILEVERSION {numeric_version},0
PRODUCTVERSION {numeric_version},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "FileDescription", "Hardware monitoring and storage analysis\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "InternalName", "rigometry\0"
            VALUE "LegalCopyright", "Copyright (c) 2026 Marlon\0"
            VALUE "OriginalFilename", "Rigometry.exe\0"
            VALUE "ProductName", "Rigometry\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#,
            icon.display().to_string().replace('\\', "/")
        ),
    )
    .expect("write application resource source");
    let compiler = resource_compiler();
    let status = Command::new(compiler)
        .args(["/nologo", "/c65001", "/fo"])
        .arg(&resource)
        .arg(&rc_file)
        .status()
        .expect("Windows SDK rc.exe is required; build from an x64 Developer PowerShell");
    assert!(
        status.success(),
        "Windows application resource compilation failed"
    );
    println!("cargo:rustc-link-arg-bin=rigometry={}", resource.display());
}

fn resource_compiler() -> PathBuf {
    if let Some(path) = env::var_os("RC") {
        return path.into();
    }
    let sdk = env::var_os("WindowsSdkDir").map(PathBuf::from).or_else(|| {
        env::var_os("ProgramFiles(x86)").map(|path| PathBuf::from(path).join("Windows Kits/10"))
    });
    if let Some(sdk) = sdk {
        // The SDK resource compiler runs on the build host, independent of the target.
        let arch = if env::var("HOST").unwrap_or_default().starts_with("aarch64") {
            "arm64"
        } else {
            "x64"
        };
        if let Ok(version) = env::var("WindowsSDKVersion") {
            let candidate = sdk
                .join("bin")
                .join(version.trim_end_matches(['\\', '/']))
                .join(arch)
                .join("rc.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
        if let Ok(entries) = fs::read_dir(sdk.join("bin")) {
            let mut versions: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            versions.sort();
            for version in versions.into_iter().rev() {
                let candidate = version.join(arch).join("rc.exe");
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    "rc.exe".into()
}
