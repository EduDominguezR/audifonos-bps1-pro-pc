#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod icon;
mod icons;
mod meta;
mod settings;
mod single;
mod theme;
mod tray;
mod widgets;

use eframe::egui;
use meta::WINDOW_TITLE;

const MAX_CRASH_BYTES: usize = 64 * 1024;

fn main() -> eframe::Result<()> {
    install_crash_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mock = args.iter().any(|arg| arg == "--mock") || std::env::var_os("BASEUS_MOCK").is_some();
    let start_hidden = args.iter().any(|arg| arg == "--tray");
    let debug = args.iter().any(|arg| arg == "--debug")
        || std::env::var_os("BASEUS_DEBUG").is_some_and(|value| value.to_string_lossy() == "1");
    baseus_transport::set_debug(debug);

    if !single::acquire() {
        single::focus_existing();
        return Ok(());
    }

    autostart::sync_path();

    let icon = icon::rgba(64);
    let viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_app_id("AudifonosBaseus")
        .with_inner_size([380.0, 640.0])
        .with_min_inner_size([380.0, 640.0])
        .with_max_inner_size([380.0, 640.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true)
        .with_icon(egui::IconData {
            rgba: icon,
            width: 64,
            height: 64,
        })
        .with_visible(!start_hidden);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: true,
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::BaseusApp::new(cc, mock, start_hidden, debug))
                as Box<dyn eframe::App>)
        }),
    )
}

fn install_crash_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic sin mensaje".to_owned());
        let location = info
            .location()
            .map(|place| format!("{}:{}:{}", place.file(), place.line(), place.column()))
            .unwrap_or_else(|| "desconocida".to_owned());
        report_crash(&format!("mensaje: {message}\nubicaci\u{f3}n: {location}\n"));
    }));
}

/// Writes a fresh crash report and shows the same dialog for panics and fatal
/// startup errors. No allocation beyond the message itself is required.
fn report_crash(details: &str) {
    write_crash_log(details);
    show_crash_dialog();
}

pub(crate) fn fatal(details: &str) -> ! {
    report_crash(&format!("Audifonos Baseus\n{details}\n"));
    std::process::exit(1);
}

fn write_crash_log(details: &str) {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return;
    };
    let path = std::path::PathBuf::from(appdata)
        .join("AudifonosBaseus")
        .join("crash.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut text = details.to_owned();
    if text.len() > MAX_CRASH_BYTES {
        let mut end = MAX_CRASH_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    let _ = std::fs::write(path, text);
}

#[cfg(windows)]
fn show_crash_dialog() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let text: Vec<u16> = "La app encontr\u{f3} un error. Detalles en crash.log"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = "Audifonos Baseus"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // The dialog is the only reliable channel in a release build: the binary
    // runs without a console because of windows_subsystem.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_crash_dialog() {
    eprintln!("La app encontr\u{f3} un error. Detalles en crash.log");
}
