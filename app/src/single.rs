//! A second launch focuses the window that is already open.

#[cfg(windows)]
mod imp {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    /// Returns false when another instance already owns the mutex.
    ///
    /// The handle is intentionally leaked: the kernel releases it when the
    /// process exits, and holding it is what keeps the name taken.
    pub fn acquire() -> bool {
        unsafe {
            match CreateMutexW(None, true, w!("Local\\AudifonosBaseus.SingleInstance")) {
                Ok(handle) => {
                    if GetLastError() == ERROR_ALREADY_EXISTS {
                        false
                    } else {
                        // Windows closes the handle when the process ends; the
                        // name stays taken for as long as we run.
                        let _ = handle;
                        true
                    }
                }
                // If the mutex cannot be created, running twice is better than
                // refusing to start at all.
                Err(_) => true,
            }
        }
    }

    pub fn focus_existing() {
        let title = windows::core::HSTRING::from(crate::meta::WINDOW_TITLE);
        unsafe {
            if let Ok(window) = FindWindowW(None, &title) {
                let _ = ShowWindow(window, SW_RESTORE);
                let _ = SetForegroundWindow(window);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn acquire() -> bool {
        true
    }

    pub fn focus_existing() {}
}

pub use imp::{acquire, focus_existing};
