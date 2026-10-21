//! Embeds the icon and the version block into the executable.
//!
//! The resource compiler is called directly instead of through a helper crate.
//! This project lives in a path with spaces, and a helper that joins a command
//! line by hand breaks there. Passing arguments one by one avoids the whole
//! class of quoting bugs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=BASEUS_SKIP_WINRES");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var_os("BASEUS_SKIP_WINRES").is_some() {
        println!("cargo:warning=Recursos de Windows omitidos a petición (BASEUS_SKIP_WINRES)");
        return;
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");

    let rc_path = out.join("app.rc");
    let source = rc_source(&manifest.join("assets").join("icon.ico"), &version);
    if let Err(error) = std::fs::write(&rc_path, source) {
        println!("cargo:warning=Sin icono ni metadatos incrustados: {error}");
        return;
    }

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let built = if target_env == "msvc" {
        compile_msvc(&rc_path, &out)
    } else {
        compile_gnu(&rc_path, &out)
    };
    if let Err(error) = built {
        // A missing resource compiler must not stop the build: the window still
        // gets an icon at runtime, only Explorer loses the pretty one.
        println!("cargo:warning=Sin icono ni metadatos incrustados: {error}");
    }
}

fn rc_source(icon: &Path, version: &str) -> String {
    // Forward slashes keep the path out of the resource compiler's escape rules.
    let icon = icon.to_string_lossy().replace('\\', "/");
    let (major, minor, patch) = version_parts(version);
    format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
FILEFLAGSMASK 0x3f
FILEFLAGS 0x0
{{
BLOCK "StringFileInfo"
{{
BLOCK "040904b0"
{{
VALUE "FileDescription", "Control para audífonos Baseus Bass BP1 Pro ANC"
VALUE "FileVersion", "{version}"
VALUE "InternalName", "AudifonosBaseus"
VALUE "LegalCopyright", "MIT. Basado en baseus-desktop."
VALUE "OriginalFilename", "AudifonosBaseus.exe"
VALUE "ProductName", "Audífonos Baseus"
VALUE "ProductVersion", "{version}"
}}
}}
BLOCK "VarFileInfo"
{{
VALUE "Translation", 0x0409, 0x04b0
}}
}}
"#
    )
}

fn version_parts(version: &str) -> (u16, u16, u16) {
    let mut parts = version.split('.').map(|part| {
        part.split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("0")
            .parse::<u16>()
            .unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn compile_gnu(rc: &Path, out: &Path) -> Result<(), String> {
    let windres = find_in_path("windres.exe").ok_or("no se encontró windres")?;
    let object = out.join("app_res.o");
    let status = Command::new(&windres)
        .args(["-O", "coff"])
        .arg("-i")
        .arg(rc)
        .arg("-o")
        .arg(&object)
        .status()
        .map_err(|error| format!("no se pudo ejecutar windres: {error}"))?;
    if !status.success() {
        return Err("windres rechazó el archivo de recursos".to_owned());
    }
    println!("cargo:rustc-link-arg-bins={}", object.display());
    Ok(())
}

fn compile_msvc(rc: &Path, out: &Path) -> Result<(), String> {
    let compiler = find_in_path("rc.exe")
        .or_else(find_sdk_rc)
        .ok_or("no se encontró rc.exe")?;
    let resource = out.join("app.res");
    let status = Command::new(&compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(rc)
        .status()
        .map_err(|error| format!("no se pudo ejecutar rc.exe: {error}"))?;
    if !status.success() {
        return Err("rc.exe rechazó el archivo de recursos".to_owned());
    }
    println!("cargo:rustc-link-arg-bins={}", resource.display());
    Ok(())
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

fn find_sdk_rc() -> Option<PathBuf> {
    let mut roots = Vec::new();
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(variable) {
            roots.push(
                PathBuf::from(root)
                    .join("Windows Kits")
                    .join("10")
                    .join("bin"),
            );
        }
    }
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        versions.sort();
        for version in versions.iter().rev() {
            let candidate = version.join("x64").join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
