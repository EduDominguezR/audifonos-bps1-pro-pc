//! "Start with Windows" through the per-user Run key.
//!
//! HKCU needs no administrator rights, which is why the app uses it instead of
//! a scheduled task or HKLM.

#[cfg(windows)]
mod imp {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "AudifonosBaseus";

    fn launch_value(exe: &std::path::Path) -> String {
        format!("\"{}\" --tray", exe.display())
    }

    pub fn is_enabled() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|key| key.get_value::<String, _>(VALUE))
            .is_ok()
    }

    /// Rewrites the entry when the exe was moved, so autostart keeps working.
    pub fn sync_path() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY) else {
            return;
        };
        if let Ok(current) = key.get_value::<String, _>(VALUE) {
            if current != launch_value(&exe) {
                let _ = set_enabled(true);
            }
        }
    }

    pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
        let key =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
        if enabled {
            let exe = std::env::current_exe()?;
            key.set_value(VALUE, &launch_value(&exe))
        } else {
            match key.delete_value(VALUE) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn sync_path() {}

    pub fn set_enabled(_enabled: bool) -> std::io::Result<()> {
        Err(std::io::Error::other("solo Windows"))
    }
}

pub use imp::{is_enabled, set_enabled, sync_path};
