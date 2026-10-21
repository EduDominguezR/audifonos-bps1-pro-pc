//! Tray icon, context menu and balloon notifications.
//!
//! A dedicated thread owns a message-only window and runs the Win32 message
//! loop. The UI thread talks to it through `PostMessageW` plus a small queue for
//! notification text, so neither side blocks on the other.

#![cfg(windows)]

use std::mem::size_of;
use std::sync::Mutex;
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON, HWND_MESSAGE,
    IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP,
    WNDCLASSW,
};

const WM_TRAYICON: u32 = WM_APP + 1;
const WM_SHOW_NOTIFICATION: u32 = WM_APP + 2;
const WM_NULL: u32 = 0;
const TRAY_ID: u32 = 1;

struct Callbacks {
    open: Box<dyn Fn() + Send>,
    quit: Box<dyn Fn() + Send>,
}

static CALLBACKS: Mutex<Option<Callbacks>> = Mutex::new(None);
static NOTIFICATIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Handle the app keeps to post balloons and to remove the icon on exit.
pub struct TrayHandle {
    window: HWND,
}

// The handle is only used to post messages; the window lives on the tray thread.
unsafe impl Send for TrayHandle {}
unsafe impl Sync for TrayHandle {}

impl TrayHandle {
    pub fn notify(&self, title: &str, body: &str) {
        if let Ok(mut queue) = NOTIFICATIONS.lock() {
            queue.push((title.to_owned(), body.to_owned()));
        }
        unsafe {
            let _ = PostMessageW(
                Some(self.window),
                WM_SHOW_NOTIFICATION,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    /// Asks the tray thread to remove the icon and exit. Best effort: the process
    /// is usually about to end anyway.
    pub fn remove(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.window), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn spawn(
    open: impl Fn() + Send + 'static,
    quit: impl Fn() + Send + 'static,
) -> Option<TrayHandle> {
    if let Ok(mut slot) = CALLBACKS.lock() {
        *slot = Some(Callbacks {
            open: Box::new(open),
            quit: Box::new(quit),
        });
    } else {
        return None;
    }

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Option<isize>>();
    std::thread::Builder::new()
        .name("baseus-tray".to_owned())
        .spawn(move || match unsafe { create_window() } {
            Some(window) => {
                let _ = ready_tx.send(Some(window.0 as isize));
                message_loop();
            }
            None => {
                let _ = ready_tx.send(None);
            }
        })
        .ok()?;

    let address = ready_rx.recv_timeout(Duration::from_secs(5)).ok()??;
    Some(TrayHandle {
        window: HWND(address as *mut core::ffi::c_void),
    })
}

unsafe fn create_window() -> Option<HWND> {
    let module = GetModuleHandleW(None).ok()?;
    let instance = HINSTANCE(module.0);
    let class = w!("AudifonosBaseusTray");

    let class_info = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassW(&class_info);

    let window = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        w!("AudifonosBaseusTray"),
        WINDOW_STYLE(0),
        0,
        0,
        0,
        0,
        Some(HWND_MESSAGE),
        None,
        Some(instance),
        None,
    )
    .ok()?;

    let mut data = icon_data(window, load_icon(instance));
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAYICON;
    data.szTip = wide("Audífonos Baseus");
    if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
        return None;
    }
    Some(window)
}

unsafe fn load_icon(instance: HINSTANCE) -> HICON {
    // MAKEINTRESOURCE(1): the build script stores the icon under id 1, and this
    // pointer is that integer id, not an address.
    #[allow(clippy::manual_dangling_ptr)]
    let id = PCWSTR(1 as *const u16);
    if let Ok(icon) = LoadIconW(Some(instance), id) {
        return icon;
    }
    LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
}

fn icon_data(window: HWND, icon: HICON) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: TRAY_ID,
        hIcon: icon,
        ..Default::default()
    }
}

fn message_loop() {
    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TRAYICON => {
            match (lparam.0 as u32) & 0xFFFF {
                WM_LBUTTONUP => run_open(),
                WM_RBUTTONUP => show_menu(window),
                _ => {}
            }
            LRESULT(0)
        }
        WM_SHOW_NOTIFICATION => {
            show_balloon(window);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(window);
            LRESULT(0)
        }
        WM_DESTROY => {
            let data = icon_data(window, HICON::default());
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn show_menu(window: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let _ = AppendMenuW(menu, MF_STRING, 1, w!("Abrir"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, 2, w!("Salir"));

    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    // Without this the menu stays on screen when the user clicks elsewhere.
    let _ = SetForegroundWindow(window);
    let selected = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
        point.x,
        point.y,
        None,
        window,
        None,
    );
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));

    match selected.0 {
        1 => run_open(),
        2 => run_quit(),
        _ => {}
    }
}

unsafe fn show_balloon(window: HWND) {
    let next = NOTIFICATIONS.lock().ok().and_then(|mut queue| {
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    });
    let Some((title, body)) = next else {
        return;
    };
    let instance = GetModuleHandleW(None)
        .map(|module| HINSTANCE(module.0))
        .unwrap_or_default();
    let mut data = icon_data(window, load_icon(instance));
    data.uFlags = NIF_INFO;
    data.szInfoTitle = wide(&title);
    data.szInfo = wide(&body);
    data.dwInfoFlags = NIIF_INFO;
    let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
}

fn run_open() {
    if let Ok(callbacks) = CALLBACKS.lock() {
        if let Some(callbacks) = callbacks.as_ref() {
            (callbacks.open)();
        }
    }
}

fn run_quit() {
    if let Ok(callbacks) = CALLBACKS.lock() {
        if let Some(callbacks) = callbacks.as_ref() {
            (callbacks.quit)();
        }
    }
}

fn wide<const N: usize>(text: &str) -> [u16; N] {
    let mut buffer = [0u16; N];
    for (slot, unit) in buffer.iter_mut().zip(text.encode_utf16()).take(N - 1) {
        *slot = unit;
    }
    buffer
}
