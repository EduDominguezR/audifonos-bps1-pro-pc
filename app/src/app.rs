use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use baseus_protocol::{
    AncMode, BatteryLevel, Command, DeviceEvent, EqPreset, ANC_MAX_LEVEL, ANC_MIN_LEVEL,
};
use baseus_transport::{Event, Request, Worker};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, Layout, Margin, Pos2, Rect, RichText, Stroke,
    TextureHandle, Ui, Vec2,
};

use crate::icons;
use crate::settings::{self, Settings};
use crate::theme;
use crate::tray::TrayHandle;
use crate::widgets;

/// How the radio is doing, as far as the UI knows.
#[derive(Clone, PartialEq)]
enum Conn {
    Scanning,
    Connected,
    NotFound,
    Disconnected,
    Failed(String),
}

#[derive(Clone, Copy, Default)]
struct Battery {
    left: Option<BatteryLevel>,
    right: Option<BatteryLevel>,
    case: Option<BatteryLevel>,
    case_charging: bool,
}

/// One animated gauge, in segments (0 to 10).
#[derive(Clone, Copy)]
struct BarAnim {
    to: f32,
    rate: f32,
    until: Instant,
}

/// The UI's optimistic EQ choice wins for 1 s, except when the device confirms it.
fn eq_event_allowed(incoming: EqPreset, guard: Option<(EqPreset, Instant)>, now: Instant) -> bool {
    guard.is_none_or(|(selected, until)| now >= until || incoming == selected)
}

/// Timed wake-ups only make sense while there is a window to paint.
fn should_schedule_wake(hidden: bool, minimized: bool) -> bool {
    !hidden && !minimized
}

pub struct BaseusApp {
    worker: Worker,
    queue: Arc<Mutex<VecDeque<Event>>>,
    conn: Conn,
    battery: Battery,
    /// `None` until the device reports a mode.
    anc: Option<AncMode>,
    /// Firmware strength byte: the source of truth. The percentage is derived.
    level_byte: u8,
    /// Sonoma's sliding ANC pill position, in option index units.
    anc_pill: f32,
    eq: EqPreset,
    /// Optimistic EQ selection stays authoritative for a short window.
    eq_guard: Option<(EqPreset, Instant)>,
    /// `None` until the device reports the switch position.
    game: Option<bool>,
    autostart: bool,
    settings: Settings,
    /// The saved ANC/EQ/game state has been pushed for this connection.
    settings_restored: bool,
    pending_style: Option<theme::Style>,
    toast: Option<(String, Instant)>,
    find_confirm: bool,
    /// `Some(deadline)` while the UI should show the tone as playing. The worker
    /// owns the real 8 s timer and the off command.
    find_until: Option<Instant>,
    pending_level: Option<Instant>,
    low_battery_notified: [bool; 2],
    /// When the last battery or case frame arrived.
    battery_updated: Option<Instant>,
    /// Lit segments, 0 to 10, one per gauge (left, right, case).
    battery_lit: [f32; 3],
    battery_anim: [Option<BarAnim>; 3],
    battery_seen: [bool; 3],
    /// Cached age label, rebuilt only when the displayed second changes.
    last_battery_text: String,
    last_battery_second: u64,
    /// The connection toast, tracked so a second one cannot stack on top.
    notification_viewport: Option<egui::ViewportId>,
    notification_opened: Option<Instant>,
    notification_pending: bool,
    toast_conexion_mostrado: bool,
    notification_dismissed: Arc<AtomicBool>,
    /// Device has not acknowledged the last ANC/EQ write yet.
    sync_until: Option<Instant>,
    /// Equalizer row split: 0 = left card wide, 1 = right card wide, 0.5 = even.
    eq_shift: [f32; 2],
    grain: TextureHandle,
    quit: Arc<AtomicBool>,
    tray: Option<TrayHandle>,
    debug: bool,
    frame_started: Option<Instant>,
    slow_frames: u16,
    hidden: Arc<AtomicBool>,
    minimized: bool,
    _bridge: std::thread::JoinHandle<()>,
}

/// Starts the radio thread. The mock is a development aid: release builds are
/// compiled without the feature and always talk to real hardware.
fn start_worker(mock: bool, events: tokio::sync::mpsc::UnboundedSender<Event>) -> Worker {
    #[cfg(feature = "mock")]
    if mock {
        return baseus_transport::spawn_mock(events);
    }
    #[cfg(not(feature = "mock"))]
    let _ = mock;

    // Keep a sender for the failure path: spawn_real consumes its own.
    let fallback = events.clone();
    match Worker::spawn_real(events) {
        Ok(worker) => worker,
        Err(error) => Worker::spawn_failed(fallback, error.to_string()),
    }
}

impl BaseusApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mock: bool,
        start_hidden: bool,
        debug: bool,
    ) -> Self {
        let settings = settings::load();
        theme::set_style(settings.style);
        theme::install_fonts(&cc.egui_ctx);
        theme::apply_style(&cc.egui_ctx);
        cc.egui_ctx.set_zoom_factor(1.0);

        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = start_worker(mock, events_tx);

        // The radio thread never touches egui. A tiny bridge moves events into a
        // queue and asks for a repaint, which is the only cross-thread wake-up.
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let bridge_queue = Arc::clone(&queue);
        let bridge_ctx = cc.egui_ctx.clone();
        let bridge = std::thread::Builder::new()
            .name("baseus-bridge".to_owned())
            .spawn(move || {
                while let Some(event) = events_rx.blocking_recv() {
                    if let Ok(mut queue) = bridge_queue.lock() {
                        queue.push_back(event);
                    }
                    bridge_ctx.request_repaint();
                }
            });
        let bridge = match bridge {
            Ok(handle) => handle,
            Err(error) => {
                crate::fatal(&format!("No se pudo iniciar el puente de eventos: {error}"))
            }
        };

        let quit = Arc::new(AtomicBool::new(false));
        let hidden = Arc::new(AtomicBool::new(start_hidden));
        let tray_hidden = Arc::clone(&hidden);
        let open_ctx = cc.egui_ctx.clone();
        let quit_ctx = cc.egui_ctx.clone();
        let quit_flag = Arc::clone(&quit);
        let tray = crate::tray::spawn(
            move || {
                tray_hidden.store(false, Ordering::Relaxed);
                open_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                open_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                open_ctx.request_repaint();
            },
            move || {
                quit_flag.store(true, Ordering::SeqCst);
                quit_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                quit_ctx.request_repaint();
            },
        );

        let settings = settings::load();
        let autostart = crate::autostart::is_enabled();
        let saved_anc = anc_from_byte(settings.anc_mode);
        let saved_eq = EqPreset::from_byte(settings.eq_preset).unwrap_or_default();
        let saved_game = settings.game_mode;

        if start_hidden && tray.is_none() {
            // Hiding without a tray icon would leave the app unreachable.
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            hidden.store(false, Ordering::Relaxed);
        }

        let grain = widgets::grain(&cc.egui_ctx);
        Self {
            worker,
            queue,
            conn: Conn::Scanning,
            battery: Battery::default(),
            anc: Some(saved_anc),
            level_byte: settings.anc_level,
            anc_pill: match saved_anc {
                AncMode::Off => 0.0,
                AncMode::Anc => 1.0,
                AncMode::Transparency => 2.0,
            },
            eq: saved_eq,
            eq_guard: None,
            game: Some(saved_game),
            autostart,
            settings,
            settings_restored: false,
            pending_style: None,
            toast: None,
            find_confirm: false,
            find_until: None,
            pending_level: None,
            low_battery_notified: [false; 2],
            battery_updated: None,
            battery_lit: [0.0; 3],
            battery_anim: [None; 3],
            battery_seen: [false; 3],
            last_battery_text: String::new(),
            last_battery_second: u64::MAX,
            notification_viewport: None,
            notification_opened: None,
            notification_pending: false,
            toast_conexion_mostrado: false,
            notification_dismissed: Arc::new(AtomicBool::new(false)),
            sync_until: None,
            eq_shift: [0.0, 0.5],
            grain,
            quit,
            tray,
            debug,
            frame_started: None,
            slow_frames: 0,
            hidden,
            minimized: false,
            _bridge: bridge,
        }
    }

    fn send(&self, command: Command) {
        self.worker.request(Request::Send(command));
    }

    fn push_anc(&mut self) {
        let Some(mode) = self.anc else {
            return;
        };
        self.send(Command::SetAnc {
            mode,
            level: self.level_byte,
        });
        self.mark_sync();
    }

    fn mark_sync(&mut self) {
        self.sync_until = Some(Instant::now() + Duration::from_secs(2));
    }

    fn show_toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn persist(&mut self) {
        if settings::save(&self.settings).is_err() {
            self.show_toast("No pude guardar los ajustes.");
        }
    }

    fn remember_anc(&mut self, mode: AncMode) {
        let byte = mode.mode_byte();
        if self.settings.anc_mode != byte {
            self.settings.anc_mode = byte;
            self.persist();
        }
    }

    fn remember_eq(&mut self, preset: EqPreset) {
        let byte = preset.to_byte();
        if self.settings.eq_preset != byte {
            self.settings.eq_preset = byte;
            self.persist();
        }
    }

    fn remember_game(&mut self, on: bool) {
        if self.settings.game_mode != on {
            self.settings.game_mode = on;
            self.persist();
        }
    }

    fn remember_level(&mut self) {
        if self.settings.anc_level != self.level_byte {
            self.settings.anc_level = self.level_byte;
            self.persist();
        }
    }

    fn start_find(&mut self) {
        self.worker
            .request(Request::FindFor(Duration::from_secs(8)));
        self.find_until = Some(Instant::now() + Duration::from_secs(8));
    }

    fn stop_find(&mut self) {
        self.worker.request(Request::StopFind);
        if self.find_until.take().is_some() {
            self.show_toast("Tono detenido.");
        }
    }

    fn drive_notification(&mut self, ctx: &egui::Context, now: Instant) {
        if let Some(id) = self.notification_viewport {
            let expired = self.notification_opened.is_some_and(|opened| {
                now.saturating_duration_since(opened) >= Duration::from_secs(4)
            });
            let dismissed = self.notification_dismissed.load(Ordering::Relaxed);
            if expired || dismissed || self.conn != Conn::Connected {
                ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Close);
                self.notification_viewport = None;
                self.notification_opened = None;
                self.notification_dismissed.store(false, Ordering::Relaxed);
            }
        }
        let mut opened_now = false;
        if self.notification_pending && self.notification_viewport.is_none() {
            self.notification_pending = false;
            self.notification_viewport =
                Some(egui::ViewportId::from_hash_of("baseus-connected-toast"));
            self.notification_opened = Some(now);
            self.notification_dismissed.store(false, Ordering::Relaxed);
            opened_now = true;
        }
        let Some(id) = self.notification_viewport else {
            return;
        };
        let battery = self.battery;
        let dismissed = Arc::clone(&self.notification_dismissed);
        ctx.show_viewport_deferred(id, notification_builder(ctx), move |ui, _| {
            notification_ui(ui, battery, &dismissed);
        });
        if opened_now {
            ctx.request_repaint_of(id);
        }
    }

    fn check_low_battery(&mut self, side: &str, index: usize, percent: u8) {
        match low_battery_action(percent, self.low_battery_notified[index]) {
            LowBattery::Rearm => self.low_battery_notified[index] = false,
            LowBattery::Notify => {
                self.low_battery_notified[index] = true;
                if !self.settings.low_battery_alerts {
                    return;
                }
                if let Some(tray) = &self.tray {
                    tray.notify(
                        "Batería baja",
                        &format!("{side}: {percent}%. Cárgalos cuando puedas."),
                    );
                }
            }
            LowBattery::Nothing => {}
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        let queue = Arc::clone(&self.queue);
        let mut queue = match queue.lock() {
            Ok(queue) => queue,
            Err(_) => return,
        };
        while let Some(event) = queue.pop_front() {
            self.apply(event, ctx);
        }
    }

    fn apply(&mut self, event: Event, ctx: &egui::Context) {
        match event {
            Event::Scanning => self.conn = Conn::Scanning,
            Event::Connected => {
                self.conn = Conn::Connected;
                self.low_battery_notified = [false; 2];
                self.anc = Some(anc_from_byte(self.settings.anc_mode));
                self.level_byte = self.settings.anc_level;
                self.eq = EqPreset::from_byte(self.settings.eq_preset).unwrap_or_default();
                self.game = Some(self.settings.game_mode);
                self.settings_restored = false;
            }
            Event::NotFound => {
                self.conn = Conn::NotFound;
                self.reset_device_state();
            }
            Event::Disconnected => {
                self.conn = Conn::Disconnected;
                self.reset_device_state();
            }
            Event::Failed(message) => {
                self.conn = Conn::Failed(message);
                self.reset_device_state();
            }
            Event::Device(device) => match device {
                DeviceEvent::Battery(state) => {
                    let now = Instant::now();
                    self.battery.left = Some(state.left);
                    self.battery.right = Some(state.right);
                    self.animate_bar(0, lit_for(Some(state.left)));
                    self.animate_bar(1, lit_for(Some(state.right)));
                    self.battery_updated = Some(now);
                    if self.conn == Conn::Connected && !self.toast_conexion_mostrado {
                        self.toast_conexion_mostrado = true;
                        if self.notification_viewport.is_none() {
                            self.notification_pending = true;
                        }
                    }
                    if let BatteryLevel::Level(percent) = state.left {
                        self.check_low_battery("Izquierdo", 0, percent);
                    }
                    if let BatteryLevel::Level(percent) = state.right {
                        self.check_low_battery("Derecho", 1, percent);
                    }
                    if !self.settings_restored {
                        let mode = anc_from_byte(self.settings.anc_mode);
                        let level = self.settings.anc_level;
                        let eq = EqPreset::from_byte(self.settings.eq_preset).unwrap_or_default();
                        let game = self.settings.game_mode;
                        self.anc = Some(mode);
                        self.level_byte = level;
                        self.eq = eq;
                        self.game = Some(game);
                        self.send(Command::SetAnc { mode, level });
                        self.send(Command::SetEq(eq));
                        self.send(Command::SetGameMode(game));
                        self.settings_restored = true;
                        ctx.request_repaint();
                    }
                }
                DeviceEvent::Case(state) => {
                    let now = Instant::now();
                    self.battery.case = Some(state.case);
                    self.battery.case_charging = state.case_charging;
                    self.animate_bar(2, lit_for(Some(state.case)));
                    self.battery_updated = Some(now);
                }
                DeviceEvent::Anc(mode) => {
                    self.anc = Some(mode);
                    self.sync_until = None;
                    if self.settings_restored {
                        self.remember_anc(mode);
                    }
                }
                DeviceEvent::GameMode(on) => {
                    self.game = Some(on);
                    if self.settings_restored {
                        self.remember_game(on);
                    }
                }
                DeviceEvent::Eq(preset) => {
                    let now = Instant::now();
                    if self.eq_guard.is_some_and(|(_, until)| now >= until) {
                        self.eq_guard = None;
                    }
                    if eq_event_allowed(preset, self.eq_guard, now) {
                        self.eq = preset;
                        self.sync_until = None;
                        if self.settings_restored {
                            self.remember_eq(preset);
                        }
                    }
                }
                DeviceEvent::Identity(_) => {}
            },
        }
    }

    fn animate_bar(&mut self, index: usize, target: f32) {
        let first = !self.battery_seen[index];
        self.battery_seen[index] = true;
        self.battery_anim[index] = Some(BarAnim {
            to: target,
            rate: if first { 5.0 } else { 18.0 },
            until: Instant::now()
                + if first {
                    Duration::from_millis(600)
                } else {
                    Duration::from_millis(150)
                },
        });
    }

    /// Nothing about the device is known once the link is gone.
    fn reset_device_state(&mut self) {
        self.battery = Battery::default();
        self.anc = None;
        self.game = None;
        self.eq_guard = None;
        self.find_until = None;
        self.battery_updated = None;
        self.battery_lit = [0.0; 3];
        self.battery_anim = [None; 3];
        self.last_battery_text.clear();
        self.last_battery_second = u64::MAX;
        self.notification_pending = false;
        self.toast_conexion_mostrado = false;
        self.settings_restored = false;
        self.sync_until = None;
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        let requested = ctx.input(|input| input.viewport().close_requested());
        if requested && !self.quit.load(Ordering::SeqCst) && self.settings.minimize_to_tray {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden.store(true, Ordering::Relaxed);
        }
    }

    fn title_bar(&mut self, ctx: &egui::Context, ui: &mut Ui, full: Rect) {
        if theme::is_sonoma() {
            self.title_bar_sonoma(ctx, ui, full);
            return;
        }
        let bar = Rect::from_min_size(
            full.min,
            Vec2::new(full.width(), theme::metrics().title_bar),
        );
        let painter = ui.painter().clone();
        painter.rect_filled(bar, 0.0, theme::pal().bg);
        painter.hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            Stroke::new(1.0, theme::pal().border),
        );
        painter.text(
            egui::pos2(bar.left() + 15.0, bar.center().y),
            Align2::CENTER_CENTER,
            icons::HEADPHONES,
            theme::icon(14.0),
            theme::pal().accent,
        );
        widgets::caps(
            ui,
            egui::pos2(bar.left() + 28.0, bar.center().y),
            "Audífonos Baseus",
            11.0,
            theme::pal().text,
            Align2::LEFT_CENTER,
        );

        let close = Rect::from_min_size(
            egui::pos2(bar.right() - 38.0, bar.top() + 2.0),
            Vec2::new(36.0, bar.height() - 4.0),
        );
        let minimize = Rect::from_min_size(
            egui::pos2(bar.right() - 76.0, bar.top() + 2.0),
            Vec2::new(36.0, bar.height() - 4.0),
        );

        let drag = ui.interact(bar, Id::unique("title-drag"), egui::Sense::click_and_drag());
        let minimize_response =
            ui.interact(minimize, Id::unique("title-min"), egui::Sense::click());
        let close_response = ui.interact(close, Id::unique("title-close"), egui::Sense::click());
        minimize_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Minimizar"));
        close_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Cerrar"));

        if minimize_response.hovered() {
            painter.rect_filled(minimize, CornerRadius::same(4), theme::pal().raised);
        }
        if close_response.hovered() {
            painter.rect_filled(close, CornerRadius::same(4), theme::pal().red);
        }
        painter.text(
            minimize.center(),
            Align2::CENTER_CENTER,
            icons::MINUS,
            theme::icon_bold(12.0),
            theme::pal().muted,
        );
        painter.text(
            close.center(),
            Align2::CENTER_CENTER,
            icons::X,
            theme::icon_bold(12.0),
            if close_response.hovered() {
                theme::pal().on_accent
            } else {
                theme::pal().muted
            },
        );
        widgets::focus_ring(ui, &minimize_response, minimize, 4);
        widgets::focus_ring(ui, &close_response, close, 4);

        let enter = |response: &egui::Response| {
            response.has_focus()
                && ui.input(|input| {
                    input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
                })
        };
        if minimize_response.clicked() || enter(&minimize_response) {
            self.minimized = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if close_response.clicked() || enter(&close_response) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }

    fn title_bar_sonoma(&mut self, ctx: &egui::Context, ui: &mut Ui, full: Rect) {
        let bar = Rect::from_min_size(
            full.min,
            Vec2::new(full.width(), theme::metrics().title_bar),
        );
        let painter = ui.painter().clone();
        let title = "Audífonos Baseus";
        painter.text(
            bar.center(),
            Align2::CENTER_CENTER,
            title,
            theme::ui(12.0),
            theme::pal().muted,
        );

        let centers = [
            egui::pos2(bar.left() + 22.0, bar.center().y),
            egui::pos2(bar.left() + 44.0, bar.center().y),
            egui::pos2(bar.left() + 66.0, bar.center().y),
        ];
        let group = Rect::from_min_max(
            egui::pos2(centers[0].x - 8.0, centers[0].y - 8.0),
            egui::pos2(centers[2].x + 8.0, centers[2].y + 8.0),
        );
        let group_hovered = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pos| group.contains(pos));

        let close_rect = Rect::from_center_size(centers[0], Vec2::splat(22.0));
        let min_rect = Rect::from_center_size(centers[1], Vec2::splat(22.0));
        let drag = ui.interact(
            bar,
            Id::unique("sonoma-title-drag"),
            egui::Sense::click_and_drag(),
        );
        let close = ui.interact(close_rect, Id::unique("sonoma-close"), egui::Sense::click());
        let minimize = ui.interact(min_rect, Id::unique("sonoma-min"), egui::Sense::click());
        close.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Cerrar"));
        minimize.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Minimizar"));

        let red = Color32::from_rgb(0xFF, 0x5F, 0x57);
        let yellow = Color32::from_rgb(0xFE, 0xBC, 0x2E);
        let green = Color32::from_rgb(0x28, 0xC8, 0x40);
        painter.circle_filled(centers[0], 6.0, red);
        painter.circle_filled(centers[1], 6.0, yellow);
        painter.circle_filled(centers[2], 6.0, green.gamma_multiply(0.38));
        if group_hovered {
            painter.text(
                centers[0],
                Align2::CENTER_CENTER,
                icons::X,
                theme::icon_bold(8.0),
                Color32::from_rgb(0x5C, 0x00, 0x00),
            );
            painter.text(
                centers[1],
                Align2::CENTER_CENTER,
                icons::MINUS,
                theme::icon_bold(8.0),
                Color32::from_rgb(0x5C, 0x3A, 0x00),
            );
        }
        widgets::focus_ring(ui, &close, close_rect, 11);
        widgets::focus_ring(ui, &minimize, min_rect, 11);

        let enter = |response: &egui::Response| {
            response.has_focus()
                && ui.input(|input| {
                    input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
                })
        };
        if close.clicked() || enter(&close) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if minimize.clicked() || enter(&minimize) {
            self.minimized = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if drag.drag_started()
            && !group.contains(ui.ctx().pointer_interact_pos().unwrap_or(Pos2::ZERO))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        if theme::is_sonoma() {
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.text(
                egui::pos2(rect.left(), rect.top()),
                Align2::LEFT_TOP,
                "Bass BP1 Pro ANC",
                theme::ui(18.0),
                theme::pal().text,
            );
            let chip = Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + 23.0),
                Vec2::new(108.0, 17.0),
            );
            self.status_chip_sonoma(ui, chip);
            return;
        }
        // Painted rather than laid out so the header keeps an exact height.
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.text(
            rect.left_top(),
            Align2::LEFT_TOP,
            "Audífonos Baseus",
            theme::ui(17.0),
            theme::pal().text,
        );
        painter.text(
            egui::pos2(rect.left(), rect.top() + 20.0),
            Align2::LEFT_TOP,
            "BASS BP1 PRO ANC",
            theme::body(11.0),
            theme::pal().muted,
        );
        let chip = Rect::from_min_size(
            egui::pos2(rect.right() - 104.0, rect.center().y - 11.0),
            Vec2::new(104.0, 22.0),
        );
        self.status_chip(ui, chip);
    }

    fn status_chip(&self, ui: &Ui, rect: Rect) {
        let (label, color) = match &self.conn {
            Conn::Scanning => ("BUSCANDO", theme::pal().muted),
            Conn::Connected => ("CONECTADO", theme::pal().accent),
            Conn::Disconnected => ("RECONECTANDO", theme::pal().muted),
            Conn::NotFound | Conn::Failed(_) => ("SIN CONEXIÓN", theme::pal().red),
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(2), theme::pal().raised);
        painter.rect_stroke(
            rect,
            CornerRadius::same(2),
            Stroke::new(1.0, theme::pal().border),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(egui::pos2(rect.left() + 9.0, rect.center().y), 2.5, color);
        painter.text(
            egui::pos2(rect.left() + 16.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::mono(11.0),
            if matches!(self.conn, Conn::Connected) {
                theme::pal().text
            } else {
                theme::pal().muted
            },
        );
    }

    fn status_chip_sonoma(&self, ui: &Ui, rect: Rect) {
        let syncing = matches!(self.conn, Conn::Connected)
            && (self.pending_level.is_some()
                || self.sync_until.is_some_and(|until| Instant::now() < until));
        let (label, color) = match &self.conn {
            Conn::Scanning => ("Buscando", theme::pal().muted),
            Conn::Connected => ("Conectado", theme::pal().green),
            Conn::Disconnected => ("Reconectando", theme::pal().muted),
            Conn::NotFound | Conn::Failed(_) => ("Sin conexión", theme::pal().red),
        };
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(9),
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 26),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80),
            ),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 3.0, color);
        painter.text(
            egui::pos2(rect.left() + 19.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::ui(11.5),
            if matches!(self.conn, Conn::Connected) {
                theme::pal().text
            } else {
                color
            },
        );
        if syncing {
            let time = ui.input(|input| input.time) as f32;
            let pulse = 0.35 + 0.65 * (0.5 + 0.5 * (time * 7.0).sin());
            painter.circle_filled(
                egui::pos2(rect.right() - 9.0, rect.center().y),
                2.5,
                theme::pal().accent.gamma_multiply(pulse),
            );
        }
    }

    fn status_screen(&mut self, ui: &mut Ui) {
        let (title, body, tone): (&str, String, Color32) = match &self.conn {
            Conn::Scanning => (
                "Buscando tus audífonos",
                "Ábrelos, mantenlos cerca y revisa que el Bluetooth esté encendido.".to_owned(),
                theme::pal().accent,
            ),
            Conn::NotFound => (
                "No los encontré",
                "Revisa que estén encendidos y que otro equipo no esté conectado a ellos."
                    .to_owned(),
                theme::pal().red,
            ),
            Conn::Disconnected => (
                "Se perdió la conexión",
                "Se alejaron o se apagaron. Lo intento otra vez en unos segundos.".to_owned(),
                theme::pal().muted,
            ),
            Conn::Failed(message) => ("No pude conectar", message.clone(), theme::pal().red),
            Conn::Connected => return,
        };

        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                if matches!(self.conn, Conn::Scanning) {
                    ui.add(egui::Spinner::new().size(20.0).color(tone));
                } else {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 9.0, tone);
                }
                ui.add_space(4.0);
                ui.label(RichText::new(title).size(16.0).color(theme::pal().text));
            });
            ui.add_space(8.0);
            ui.label(RichText::new(body).size(13.0).color(theme::pal().muted));
            ui.add_space(16.0);

            let scanning = matches!(self.conn, Conn::Scanning);
            let (label, kind) = if scanning {
                ("Buscando...", widgets::Kind::Secondary)
            } else {
                ("Buscar de nuevo", widgets::Kind::Primary)
            };
            if widgets::wide_button(ui, label, Vec2::new(ui.available_width(), 44.0), kind)
                && !scanning
            {
                self.conn = Conn::Scanning;
                self.worker.request(Request::Rescan);
            }

            if matches!(self.conn, Conn::NotFound | Conn::Failed(_)) {
                ui.add_space(8.0);
                if widgets::wide_button(
                    ui,
                    "Revisar Bluetooth en Windows",
                    Vec2::new(ui.available_width(), 44.0),
                    widgets::Kind::Secondary,
                ) {
                    open_bluetooth_settings();
                }
            }
        });
    }

    fn battery_card(&mut self, ui: &mut Ui) {
        let updated = self.battery_updated;
        let meta = updated.map(|updated| {
            let seconds = updated.elapsed().as_secs();
            if seconds != self.last_battery_second {
                self.last_battery_second = seconds;
                self.last_battery_text = if seconds < 60 {
                    format!("hace {seconds} s")
                } else {
                    format!("hace {} min", seconds / 60)
                };
            }
            self.last_battery_text.clone()
        });
        if theme::is_sonoma() {
            let width = (ui.available_width() - 16.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::battery_tile(
                    ui,
                    width,
                    "Izquierdo",
                    self.battery.left,
                    false,
                    true,
                    self.battery_lit[0],
                );
                widgets::battery_tile(
                    ui,
                    width,
                    "Derecho",
                    self.battery.right,
                    false,
                    true,
                    self.battery_lit[1],
                );
                widgets::battery_tile(
                    ui,
                    width,
                    "Estuche",
                    self.battery.case,
                    self.battery.case_charging,
                    false,
                    self.battery_lit[2],
                );
            });
            let (row, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 13.0), egui::Sense::hover());
            if let Some(meta) = meta {
                ui.painter().text(
                    row.right_center(),
                    Align2::RIGHT_CENTER,
                    format!("Actualizado {meta}"),
                    theme::ui(11.5),
                    theme::pal().muted,
                );
            }
            return;
        }
        widgets::card(ui, |ui| {
            widgets::section(ui, "Batería", meta.as_deref());
            ui.add_space(3.0);
            let width = (ui.available_width() - 16.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::battery_tile(
                    ui,
                    width,
                    "Izquierdo",
                    self.battery.left,
                    false,
                    true,
                    self.battery_lit[0],
                );
                widgets::battery_tile(
                    ui,
                    width,
                    "Derecho",
                    self.battery.right,
                    false,
                    true,
                    self.battery_lit[1],
                );
                widgets::battery_tile(
                    ui,
                    width,
                    "Estuche",
                    self.battery.case,
                    self.battery.case_charging,
                    false,
                    self.battery_lit[2],
                );
            });
        });
    }

    fn anc_card(&mut self, ui: &mut Ui) {
        if theme::is_sonoma() {
            self.anc_card_sonoma(ui);
            return;
        }
        widgets::card(ui, |ui| {
            // No invented state: until the device answers, the card says so and
            // no mode button is lit.
            widgets::section(
                ui,
                "Cancelación de ruido",
                if self.anc.is_none() {
                    Some("Sin datos aún")
                } else {
                    None
                },
            );
            ui.add_space(3.0);
            let gap = 8.0;
            let width = (ui.available_width() - gap * 2.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for mode in [AncMode::Off, AncMode::Anc, AncMode::Transparency] {
                    let selected = self.anc == Some(mode);
                    let (icon, label) = match mode {
                        AncMode::Off => (icons::SPEAKER_SLASH, mode.label()),
                        AncMode::Anc => (icons::HEADPHONES, mode.label()),
                        AncMode::Transparency => (icons::ARROWS_LEFT_RIGHT, mode.label()),
                    };
                    if anc_button(ui, width, icon, label, selected) {
                        self.anc = Some(mode);
                        self.remember_anc(mode);
                        self.push_anc();
                        ui.ctx().request_repaint();
                    }
                }
            });
            ui.add_space(3.0);
            // The row is always present so the card never changes height; it
            // only lights up for noise cancellation.
            let enabled = self.anc == Some(AncMode::Anc);
            let dimmed = theme::pal().muted.gamma_multiply(0.7);
            ui.horizontal(|ui| {
                let (label_rect, _) =
                    ui.allocate_exact_size(Vec2::new(82.0, 24.0), egui::Sense::hover());
                widgets::caps(
                    ui,
                    label_rect.left_center(),
                    "Intensidad",
                    11.0,
                    if enabled { theme::pal().muted } else { dimmed },
                    Align2::LEFT_CENTER,
                );
                ui.spacing_mut().item_spacing.x = 6.0;
                let value_width = 34.0;
                let slider_width = (ui.available_width() - value_width - 6.0).max(90.0);
                let mut percent = percent_from_byte(self.level_byte);
                let response = ui
                    .allocate_ui_with_layout(
                        Vec2::new(slider_width, 24.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_width(slider_width);
                            widgets::level_slider(ui, &mut percent, enabled)
                        },
                    )
                    .inner;
                if response.changed() {
                    self.level_byte = byte_from_percent(percent);
                    self.pending_level = Some(Instant::now());
                }
                let shown = percent_from_byte(self.level_byte);
                ui.label(
                    RichText::new(format!("{shown}%"))
                        .font(theme::mono(12.0))
                        .color(if enabled { theme::pal().accent } else { dimmed }),
                );
            });
        });
    }

    fn anc_card_sonoma(&mut self, ui: &mut Ui) {
        widgets::card(ui, |ui| {
            let (head, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), egui::Sense::hover());
            widgets::caps(
                ui,
                head.left_center(),
                "Cancelación de ruido",
                11.0,
                theme::pal().muted,
                Align2::LEFT_CENTER,
            );
            if self.anc == Some(AncMode::Anc) {
                ui.painter().text(
                    head.right_center(),
                    Align2::RIGHT_CENTER,
                    "Cancelación activa",
                    theme::ui(11.5),
                    theme::pal().accent_sound,
                );
            }
            ui.add_space(6.0);
            let selected = self.anc.map(|mode| match mode {
                AncMode::Off => 0,
                AncMode::Anc => 1,
                AncMode::Transparency => 2,
            });
            let chosen = widgets::segmented(
                ui,
                Id::unique("anc-sonoma"),
                &["Apagado", "Cancelación", "Transparencia"],
                selected,
                self.anc_pill,
                Vec2::new(ui.available_width(), 32.0),
            );
            if let Some(index) = chosen {
                let mode = match index {
                    0 => AncMode::Off,
                    1 => AncMode::Anc,
                    _ => AncMode::Transparency,
                };
                self.anc = Some(mode);
                self.remember_anc(mode);
                self.push_anc();
                ui.ctx().request_repaint();
            }

            ui.add_space(8.0);
            let enabled = self.anc == Some(AncMode::Anc);
            let dimmed = theme::pal().muted.gamma_multiply(0.7);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (label, _) =
                    ui.allocate_exact_size(Vec2::new(66.0, 28.0), egui::Sense::hover());
                ui.painter().text(
                    label.left_center(),
                    Align2::LEFT_CENTER,
                    "Intensidad",
                    theme::ui(12.0),
                    if enabled { theme::pal().muted } else { dimmed },
                );
                let value_width = 36.0;
                let slider_width = (ui.available_width() - value_width - 4.0).max(90.0);
                let mut percent = percent_from_byte(self.level_byte);
                let response = ui
                    .allocate_ui_with_layout(
                        Vec2::new(slider_width, 28.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_width(slider_width);
                            widgets::level_slider(ui, &mut percent, enabled)
                        },
                    )
                    .inner;
                if response.changed() {
                    self.level_byte = byte_from_percent(percent);
                    self.pending_level = Some(Instant::now());
                }
                let shown = percent_from_byte(self.level_byte);
                ui.painter().text(
                    egui::pos2(ui.max_rect().right(), label.center().y),
                    Align2::RIGHT_CENTER,
                    format!("{shown}%"),
                    theme::ui(12.0),
                    if enabled { theme::pal().text } else { dimmed },
                );
            });
        });
    }

    fn eq_card(&mut self, ui: &mut Ui) {
        let sonoma = theme::is_sonoma();
        widgets::card(ui, |ui| {
            if sonoma {
                widgets::section(ui, "Perfil de sonido (EQ)", Some(self.eq.label()));
            } else {
                widgets::section(ui, "Ecualizador", None);
            }
            ui.add_space(if sonoma { 4.0 } else { 3.0 });
            let gap = 8.0;
            let base = ui.available_width() - gap;
            let narrow = base / 2.12;
            let wide = base - narrow;
            for row in 0..2 {
                if row > 0 {
                    ui.add_space(if sonoma { 4.0 } else { 3.0 });
                }
                let shift = if sonoma { 0.5 } else { self.eq_shift[row] };
                let left_width = narrow + (wide - narrow) * (1.0 - shift);
                let right_width = base - left_width;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (slot, preset) in EqPreset::ALL[row * 2..row * 2 + 2].iter().enumerate() {
                        let width = if slot == 0 { left_width } else { right_width };
                        let selected = self.eq == *preset;
                        if widgets::eq_choice(
                            ui,
                            preset.label(),
                            eq_hint(*preset),
                            eq_icon(*preset),
                            &eq_curve(*preset),
                            selected,
                            Vec2::new(width, if sonoma { 37.0 } else { 39.0 }),
                        ) {
                            self.eq = *preset;
                            self.remember_eq(*preset);
                            self.eq_guard =
                                Some((*preset, Instant::now() + Duration::from_secs(1)));
                            self.send(Command::SetEq(*preset));
                            self.mark_sync();
                            ui.ctx().request_repaint();
                        }
                    }
                });
            }
        });
    }

    fn actions_card(&mut self, ui: &mut Ui) {
        if theme::is_sonoma() {
            self.actions_card_sonoma(ui);
            return;
        }
        widgets::card(ui, |ui| {
            let game_hint = if self.game.is_some() {
                Some("Menos latencia")
            } else {
                Some("Sin datos aún")
            };
            if let Some(next) = widgets::toggle_row(
                ui,
                Id::unique("game"),
                "Modo juego",
                self.game.unwrap_or(false),
                game_hint,
            ) {
                self.game = Some(next);
                self.remember_game(next);
                self.send(Command::SetGameMode(next));
                ui.ctx().request_repaint();
            }
            ui.add_space(3.0);

            let playing = self.find_until.is_some();
            let label = if playing {
                "Detener el tono"
            } else {
                "Encontrar mis audífonos"
            };
            let kind = if playing {
                widgets::Kind::Tonal
            } else {
                widgets::Kind::Secondary
            };
            if widgets::wide_button(ui, label, Vec2::new(ui.available_width(), 36.0), kind) {
                if playing {
                    self.stop_find();
                } else {
                    self.find_confirm = true;
                }
            }
        });
    }

    fn actions_card_sonoma(&mut self, ui: &mut Ui) {
        widgets::card(ui, |ui| {
            let row = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 47.0));
            let response = ui.interact(row, Id::unique("game-sonoma"), egui::Sense::click());
            let on = self.game.unwrap_or(false);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, "Modo juego")
            });
            let painter = ui.painter();
            let box_rect = Rect::from_center_size(
                egui::pos2(row.left() + 18.0, row.center().y),
                Vec2::splat(32.0),
            );
            painter.rect_filled(
                box_rect,
                CornerRadius::same(9),
                if on {
                    theme::pal().accent.gamma_multiply(0.18)
                } else {
                    theme::pal().track
                },
            );
            painter.text(
                box_rect.center(),
                Align2::CENTER_CENTER,
                icons::GAME_CONTROLLER,
                theme::icon(17.0),
                if on {
                    theme::pal().accent
                } else {
                    theme::pal().muted
                },
            );
            painter.text(
                egui::pos2(row.left() + 41.0, row.center().y - 7.0),
                Align2::LEFT_BOTTOM,
                "Modo juego",
                theme::ui(13.0),
                theme::pal().text,
            );
            painter.text(
                egui::pos2(row.left() + 41.0, row.center().y + 6.0),
                Align2::LEFT_TOP,
                if self.game.is_some() {
                    "Menos latencia"
                } else {
                    "Sin datos aún"
                },
                theme::ui(11.5),
                theme::pal().muted,
            );
            let switch = Rect::from_min_size(
                egui::pos2(row.right() - 44.0, row.center().y - 13.0),
                Vec2::new(44.0, 26.0),
            );
            painter.rect_filled(
                switch,
                CornerRadius::same(13),
                if on {
                    theme::pal().accent
                } else {
                    theme::pal().track
                },
            );
            painter.rect_stroke(
                switch,
                CornerRadius::same(13),
                Stroke::new(1.0, Color32::from_white_alpha(18)),
                egui::StrokeKind::Inside,
            );
            let thumb = egui::pos2(
                if on {
                    switch.right() - 14.0
                } else {
                    switch.left() + 14.0
                },
                switch.center().y,
            );
            painter.circle_filled(
                thumb + Vec2::new(0.0, 0.5),
                11.0,
                Color32::from_black_alpha(50),
            );
            painter.circle_filled(thumb, 10.0, Color32::WHITE);
            widgets::focus_ring(ui, &response, row, 12);
            let activated = response.clicked()
                || (response.has_focus()
                    && ui.input(|input| {
                        input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
                    }));
            if activated {
                self.game = Some(!on);
                self.remember_game(!on);
                self.send(Command::SetGameMode(!on));
                ui.ctx().request_repaint();
            }
            ui.allocate_rect(row, egui::Sense::hover());

            ui.add_space(4.0);
            let playing = self.find_until.is_some();
            let label = if playing {
                "Detener el tono"
            } else {
                "Encontrar mis audífonos"
            };
            let kind = if playing {
                widgets::Kind::Tonal
            } else {
                widgets::Kind::Secondary
            };
            if widgets::wide_button(ui, label, Vec2::new(ui.available_width(), 36.0), kind) {
                if playing {
                    self.stop_find();
                } else {
                    self.find_confirm = true;
                }
            }
        });
    }

    fn settings_card(&mut self, ui: &mut Ui) {
        widgets::card(ui, |ui| {
            let index = match self.settings.style {
                theme::Style::HiFi => 0,
                theme::Style::Sonoma => 1,
            };
            let row_height = if theme::is_sonoma() { 28.0 } else { 30.0 };
            let mut chosen = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new((ui.available_width() - 140.0).max(0.0), row_height),
                    egui::Sense::hover(),
                );
                ui.painter().text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    "Estilo",
                    theme::ui(12.5),
                    theme::pal().text,
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(140.0, row_height),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        ui.set_width(140.0);
                        chosen = widgets::segmented(
                            ui,
                            Id::unique("style"),
                            &["Hi-Fi", "Sonoma"],
                            Some(index),
                            index as f32,
                            Vec2::new(140.0, 24.0),
                        );
                    },
                );
            });
            if let Some(next) = chosen {
                let style = if next == 0 {
                    theme::Style::HiFi
                } else {
                    theme::Style::Sonoma
                };
                if style != self.settings.style {
                    self.settings.style = style;
                    self.persist();
                    self.pending_style = Some(style);
                    ui.ctx().request_repaint();
                }
            }
            if let Some(next) = widgets::toggle_row(
                ui,
                Id::unique("autostart"),
                "Iniciar con Windows",
                self.autostart,
                None,
            ) {
                match crate::autostart::set_enabled(next) {
                    Ok(()) => {
                        self.autostart = next;
                        ui.ctx().request_repaint();
                    }
                    Err(_) => self.show_toast("No pude cambiar el inicio automático."),
                }
            }
            if let Some(next) = widgets::toggle_row(
                ui,
                Id::unique("tray"),
                "Minimizar a la bandeja",
                self.settings.minimize_to_tray,
                None,
            ) {
                self.settings.minimize_to_tray = next;
                self.persist();
                ui.ctx().request_repaint();
            }
            if let Some(next) = widgets::toggle_row(
                ui,
                Id::unique("low-battery"),
                "Aviso de batería baja",
                self.settings.low_battery_alerts,
                None,
            ) {
                self.settings.low_battery_alerts = next;
                self.persist();
                ui.ctx().request_repaint();
            }
        });
    }

    fn toast_overlay(&self, ctx: &egui::Context) {
        let Some((text, started)) = &self.toast else {
            return;
        };
        let elapsed = started.elapsed().as_secs_f32();
        let opacity = if elapsed > 3.85 {
            (1.0 - (elapsed - 3.85) / 0.15).clamp(0.0, 1.0)
        } else {
            1.0
        };
        egui::Area::new(Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -12.0))
            .interactable(false)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                egui::Frame::default()
                    .fill(theme::pal().raised)
                    .stroke(Stroke::new(1.0, theme::pal().border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).size(12.5).color(theme::pal().text));
                    });
            });
    }

    fn find_modal(&mut self, ctx: &egui::Context) {
        if !self.find_confirm {
            return;
        }
        let mut close = false;
        let modal = egui::Modal::new(Id::unique("find-modal"))
            .frame(
                egui::Frame::default()
                    .fill(theme::pal().surface)
                    .stroke(Stroke::new(1.0, theme::pal().border))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.set_width(292.0);
                ui.label(
                    RichText::new("Encontrar mis audífonos")
                        .font(theme::ui(16.0))
                        .color(theme::pal().text),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Los audífonos sonarán unos segundos. Si están en el estuche, ábrelo primero.",
                    )
                    .size(12.5)
                    .color(theme::pal().muted),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if widgets::wide_button(
                        ui,
                        "Cancelar",
                        Vec2::new(128.0, 40.0),
                        widgets::Kind::Secondary,
                    ) {
                        close = true;
                    }
                    if widgets::wide_button(
                        ui,
                        "Reproducir tono",
                        Vec2::new(156.0, 40.0),
                        widgets::Kind::Primary,
                    ) {
                        self.start_find();
                        close = true;
                    }
                });
            });
        if modal.should_close() {
            close = true;
        }
        if close {
            self.find_confirm = false;
        }
    }
}

impl eframe::App for BaseusApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.debug {
            self.frame_started = Some(Instant::now());
        }
        if let Some(style) = self.pending_style.take() {
            theme::set_style(style);
            theme::apply_style(ctx);
            ctx.request_repaint();
        }
        let minimized = ctx
            .input(|input| input.viewport().minimized)
            .unwrap_or(false);
        let was_minimized = self.minimized;
        self.minimized = minimized;
        if was_minimized && !minimized && !self.hidden.load(Ordering::Relaxed) {
            ctx.request_repaint();
        }
        self.drain_events(ctx);

        let now = Instant::now();
        let dt = ctx.input(|input| input.stable_dt).clamp(0.0001, 0.1);
        let k = 1.0 - (-dt * 18.0).exp();
        let mut wake: Option<Duration> = None;

        // Battery gauges approach their target with a frame-rate independent curve.
        for index in 0..3 {
            if let Some(anim) = self.battery_anim[index] {
                let delta = anim.to - self.battery_lit[index];
                if delta.abs() <= 0.001 || now >= anim.until {
                    self.battery_lit[index] = anim.to;
                    self.battery_anim[index] = None;
                } else {
                    self.battery_lit[index] += delta * (1.0 - (-dt * anim.rate).exp());
                    wake = Some(Duration::from_millis(16));
                }
            }
        }

        // Equalizer cards swap width over ~150 ms.
        for row in 0..2 {
            let left = EqPreset::ALL[row * 2];
            let right = EqPreset::ALL[row * 2 + 1];
            let target = if self.eq == left {
                0.0
            } else if self.eq == right {
                1.0
            } else {
                0.5
            };
            let shift = &mut self.eq_shift[row];
            let delta = target - *shift;
            if delta.abs() > 0.004 && !theme::is_sonoma() {
                *shift += delta * k;
                wake = Some(Duration::from_millis(16));
            } else if !theme::is_sonoma() {
                *shift = target;
            }
        }

        // Sonoma's segmented pill slides in ~150 ms.
        let pill_target = self.anc.map(|mode| match mode {
            AncMode::Off => 0.0,
            AncMode::Anc => 1.0,
            AncMode::Transparency => 2.0,
        });
        if let Some(target) = pill_target {
            let delta = target - self.anc_pill;
            if delta.abs() > 0.004 {
                self.anc_pill += delta * k;
                wake = Some(Duration::from_millis(16));
            } else {
                self.anc_pill = target;
            }
        }

        if let Some(until) = self.sync_until {
            if now >= until {
                self.sync_until = None;
            } else {
                wake = Some(match wake {
                    Some(current) => current.min(Duration::from_millis(60)),
                    None => Duration::from_millis(60),
                });
            }
        }
        if let Some(started) = self.pending_level {
            if now.duration_since(started) >= Duration::from_millis(160) {
                self.pending_level = None;
                self.remember_level();
                self.push_anc();
            } else {
                wake = Some(match wake {
                    Some(current) => current.min(Duration::from_millis(60)),
                    None => Duration::from_millis(60),
                });
            }
        }
        if let Some(deadline) = self.find_until {
            // The transport stops the tone on its own; this only clears the label.
            if now >= deadline {
                self.find_until = None;
            } else {
                wake = Some(match wake {
                    Some(current) => current.min(Duration::from_millis(60)),
                    None => Duration::from_millis(60),
                });
            }
        }
        if let Some((_, started)) = &self.toast {
            if now.duration_since(*started) >= Duration::from_secs(4) {
                self.toast = None;
            } else {
                wake = Some(match wake {
                    Some(current) => current.min(Duration::from_millis(60)),
                    None => Duration::from_millis(60),
                });
            }
        }
        // The "hace X s" note only needs one wake per second, and only while a
        // battery reading is on screen.
        if self.battery_updated.is_some() && self.conn == Conn::Connected {
            let elapsed = self
                .battery_updated
                .map(|updated| updated.elapsed().as_secs_f32())
                .unwrap_or(0.0);
            let next_second = (1.0 - elapsed.fract()).clamp(0.08, 1.0);
            wake = Some(match wake {
                Some(current) => current.min(Duration::from_secs_f32(next_second)),
                None => Duration::from_secs_f32(next_second),
            });
        }
        self.drive_notification(ctx, now);
        let notification_wake = self.notification_opened.map(|opened| {
            Duration::from_secs(4)
                .saturating_sub(now.saturating_duration_since(opened))
                .max(Duration::from_millis(16))
        });
        if let Some(wake) = wake {
            if should_schedule_wake(self.hidden.load(Ordering::Relaxed), self.minimized) {
                ctx.request_repaint_after(wake);
            }
        }
        if let Some(wake) = notification_wake {
            // The toast lives in its own viewport, so it must keep its timer
            // even when the main window is hidden in the tray.
            ctx.request_repaint_after(wake);
        }

        self.handle_close(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let full = ui.max_rect();
        let sonoma = theme::is_sonoma();
        if sonoma {
            ui.painter()
                .rect_filled(full.shrink(0.5), CornerRadius::same(14), theme::pal().bg);
        } else {
            ui.painter().rect_filled(full, 0.0, theme::pal().bg);
            widgets::paint_grain(ui, &self.grain, full);
        }
        let ctx = ui.ctx().clone();
        self.title_bar(&ctx, ui, full);

        let content = Rect::from_min_max(
            Pos2::new(
                full.left() + 14.0,
                full.top() + theme::metrics().title_bar + 4.0,
            ),
            Pos2::new(full.right() - 14.0, full.bottom() - 4.0),
        );

        let gap = if sonoma { 4.0 } else { 2.0 };
        let _inner = ui
            .scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(content.width());
                self.header(ui);
                ui.add_space(if sonoma { 4.0 } else { 2.0 });
                if self.conn == Conn::Connected {
                    self.battery_card(ui);
                    ui.add_space(gap);
                    self.anc_card(ui);
                    ui.add_space(gap);
                    self.eq_card(ui);
                    ui.add_space(gap);
                    self.actions_card(ui);
                    ui.add_space(gap);
                    self.settings_card(ui);
                } else {
                    self.status_screen(ui);
                }
            })
            .response;

        ui.painter().rect_stroke(
            full.shrink(0.5),
            CornerRadius::same(if sonoma { 14 } else { 0 }),
            Stroke::new(1.0, theme::pal().border),
            egui::StrokeKind::Inside,
        );

        self.toast_overlay(&ctx);
        self.find_modal(&ctx);

        if self.debug {
            if let Some(started) = self.frame_started {
                let millis = started.elapsed().as_secs_f32() * 1000.0;
                if millis > 20.0 && self.slow_frames < 200 {
                    self.slow_frames += 1;
                    baseus_transport::debug::note(&format!("update {millis:.1} ms"));
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Ask the worker to silence the tone and give it a moment to write the
        // off command before the process goes away.
        self.worker.request(Request::StopFind);
        std::thread::sleep(Duration::from_millis(150));
        if let Some(tray) = &self.tray {
            tray.remove();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Both the main window and the toast paint their own background; the
        // toast needs transparent corners, and Hi-Fi still fills its root.
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn eq_hint(preset: EqPreset) -> &'static str {
    match preset {
        EqPreset::Balanced => "Para todo",
        EqPreset::BassBoost => "Más graves",
        EqPreset::Voice => "Voces claras",
        EqPreset::Clear => "Más detalle",
    }
}

fn eq_icon(preset: EqPreset) -> &'static str {
    match preset {
        EqPreset::Balanced => icons::EQUALIZER,
        EqPreset::BassBoost => icons::WAVE_SINE,
        EqPreset::Voice => icons::MICROPHONE,
        EqPreset::Clear => icons::SPARKLE,
    }
}

/// Faint response curves, low frequencies on the left.
fn eq_curve(preset: EqPreset) -> [f32; 8] {
    match preset {
        EqPreset::Balanced => [0.48, 0.5, 0.52, 0.5, 0.5, 0.52, 0.5, 0.48],
        EqPreset::BassBoost => [0.92, 0.86, 0.62, 0.44, 0.38, 0.42, 0.5, 0.56],
        EqPreset::Voice => [0.3, 0.34, 0.58, 0.84, 0.9, 0.68, 0.5, 0.42],
        EqPreset::Clear => [0.42, 0.36, 0.4, 0.5, 0.62, 0.84, 0.94, 0.9],
    }
}

fn lit_for(level: Option<BatteryLevel>) -> f32 {
    level
        .and_then(|level| level.percent())
        .map(|percent| f32::from(percent) / 10.0)
        .unwrap_or(0.0)
}

/// One noise-control mode: icon on top, label below, amber when selected.
fn anc_button(ui: &mut Ui, width: f32, icon: &str, label: &str, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 36.0), egui::Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label));

    let hovered = response.hovered();
    let (fill, border, fg) = if selected {
        (
            theme::pal().accent,
            theme::pal().accent,
            theme::pal().on_accent,
        )
    } else if hovered {
        (
            theme::pal().raised_hover,
            theme::pal().accent,
            theme::pal().text,
        )
    } else {
        (theme::pal().raised, theme::pal().border, theme::pal().text)
    };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::metrics().radius_small),
        fill,
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::metrics().radius_small),
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 5.0),
        Align2::CENTER_TOP,
        icon,
        theme::icon_bold(14.0),
        if selected {
            theme::pal().on_accent
        } else {
            theme::pal().muted
        },
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 19.0),
        Align2::CENTER_TOP,
        label,
        theme::ui(12.0),
        fg,
    );

    widgets::focus_ring(ui, &response, rect, theme::metrics().radius_small);
    let keyboard = response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    response.clicked() || keyboard
}

/// Rounded mapping from a 0..=100 slider value to the firmware's strength byte.
/// The result is never `0xFF`: that byte means "no strength" to the firmware.
/// Maps the persisted wire byte back to a mode, ignoring unknown values.
fn anc_from_byte(byte: u8) -> AncMode {
    match byte {
        1 => AncMode::Anc,
        2 => AncMode::Transparency,
        _ => AncMode::Off,
    }
}

fn byte_from_percent(percent: u8) -> u8 {
    let percent = percent.min(100) as u32;
    let span = (ANC_MAX_LEVEL - ANC_MIN_LEVEL) as u32;
    (ANC_MIN_LEVEL as u32 + (percent * span + 50) / 100) as u8
}

/// Rounded mapping from a firmware strength byte to a 0..=100 display value.
fn percent_from_byte(byte: u8) -> u8 {
    let byte = byte.clamp(ANC_MIN_LEVEL, ANC_MAX_LEVEL);
    let span = (ANC_MAX_LEVEL - ANC_MIN_LEVEL) as u32;
    (((byte - ANC_MIN_LEVEL) as u32 * 100 + span / 2) / span) as u8
}

/// What a battery reading should do to the "already warned" flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LowBattery {
    /// Nothing to do.
    Nothing,
    /// Crossed the threshold: warn once.
    Notify,
    /// Recovered: allow a future warning.
    Rearm,
}

fn low_battery_action(percent: u8, already_warned: bool) -> LowBattery {
    if percent == 0 {
        // 0 % is what the device reports while charging or without a reading.
        return LowBattery::Nothing;
    }
    if percent > 20 {
        return LowBattery::Rearm;
    }
    if percent > 15 || already_warned {
        return LowBattery::Nothing;
    }
    LowBattery::Notify
}

fn notification_builder(ctx: &egui::Context) -> egui::ViewportBuilder {
    let monitor = ctx
        .input(|input| input.viewport().monitor_size)
        .unwrap_or(Vec2::new(1920.0, 1080.0));
    let position = Pos2::new(
        (monitor.x - 300.0 - 16.0).max(0.0),
        (monitor.y - 110.0 - 52.0).max(0.0),
    );
    egui::ViewportBuilder::default()
        .with_title("AudifonosBaseus")
        .with_inner_size([300.0, 110.0])
        .with_position(position)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_resizable(false)
        .with_active(false)
        .with_taskbar(false)
}

/// The little AirPods-style card: identity, link state and the three real
/// battery readings that arrived on this connection.
fn notification_ui(ui: &mut Ui, battery: Battery, dismissed: &Arc<AtomicBool>) {
    let full = ui.max_rect();
    let panel = full.shrink(0.5);
    let painter = ui.painter();
    painter.rect_filled(panel, CornerRadius::same(16), theme::pal().surface);
    painter.rect_stroke(
        panel,
        CornerRadius::same(16),
        Stroke::new(1.0, theme::pal().border),
        egui::StrokeKind::Inside,
    );

    let response = ui.interact(full, Id::unique("connection-toast"), egui::Sense::click());
    let close_requested = ui.input(|input| input.viewport().close_requested());
    if response.clicked() || close_requested {
        dismissed.store(true, Ordering::Relaxed);
        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
    }

    painter.text(
        egui::pos2(full.left() + 33.0, full.top() + 31.0),
        Align2::CENTER_CENTER,
        icons::HEADPHONES,
        theme::icon(30.0),
        theme::pal().accent,
    );
    painter.text(
        egui::pos2(full.left() + 60.0, full.top() + 24.0),
        Align2::LEFT_CENTER,
        "Bass BP1 Pro ANC",
        theme::ui(13.5),
        theme::pal().text,
    );
    let status = if theme::is_sonoma() {
        theme::pal().green
    } else {
        theme::pal().accent
    };
    painter.circle_filled(
        egui::pos2(full.left() + 62.0, full.top() + 44.0),
        2.0,
        status,
    );
    painter.text(
        egui::pos2(full.left() + 69.0, full.top() + 44.0),
        Align2::LEFT_CENTER,
        "Conectado",
        theme::ui(12.0),
        status,
    );
    let divider_y = full.top() + 57.0;
    painter.line_segment(
        [
            egui::pos2(full.left() + 14.0, divider_y),
            egui::pos2(full.right() - 14.0, divider_y),
        ],
        Stroke::new(1.0, theme::pal().border),
    );

    let gap = 8.0;
    let column = (full.width() - 28.0 - gap * 2.0) / 3.0;
    let readings = [
        ("Izquierdo", battery.left),
        ("Derecho", battery.right),
        ("Estuche", battery.case),
    ];
    for (index, (label, level)) in readings.into_iter().enumerate() {
        let left = full.left() + 14.0 + index as f32 * (column + gap);
        painter.text(
            egui::pos2(left, full.top() + 62.0),
            Align2::LEFT_TOP,
            label,
            theme::ui(12.0),
            theme::pal().muted,
        );
        let percent = level.and_then(|level| level.percent());
        let text = percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "—".to_owned());
        painter.text(
            egui::pos2(left, full.top() + 76.0),
            Align2::LEFT_TOP,
            text,
            theme::mono_medium(15.0),
            percent
                .map(theme::battery_color)
                .unwrap_or(theme::pal().muted),
        );
        let track =
            Rect::from_min_size(egui::pos2(left, full.top() + 97.0), Vec2::new(column, 3.0));
        painter.rect_filled(track, CornerRadius::same(2), theme::pal().track);
        if let Some(percent) = percent {
            let filled = Rect::from_min_size(
                track.min,
                Vec2::new(column * f32::from(percent) / 100.0, track.height()),
            );
            painter.rect_filled(filled, CornerRadius::same(2), theme::battery_color(percent));
        }
    }
}

#[cfg(windows)]
fn open_bluetooth_settings() {
    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:bluetooth"),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(not(windows))]
fn open_bluetooth_settings() {}

#[cfg(test)]
mod tests {
    use super::*;
    use baseus_protocol::ANC_DEFAULT_LEVEL;

    #[test]
    fn default_level_survives_a_round_trip() {
        assert_eq!(percent_from_byte(ANC_DEFAULT_LEVEL), 37);
        assert_eq!(byte_from_percent(37), ANC_DEFAULT_LEVEL);
    }

    #[test]
    fn level_minimum_round_trips() {
        assert_eq!(percent_from_byte(ANC_MIN_LEVEL), 0);
        assert_eq!(byte_from_percent(0), ANC_MIN_LEVEL);
    }

    #[test]
    fn level_maximum_never_reaches_ff() {
        assert_eq!(percent_from_byte(ANC_MAX_LEVEL), 100);
        assert_eq!(byte_from_percent(100), ANC_MAX_LEVEL);
        assert_ne!(ANC_MAX_LEVEL, 0xFF);
    }

    #[test]
    fn every_percentage_stays_below_ff() {
        for percent in 0..=100u8 {
            assert_ne!(byte_from_percent(percent), 0xFF);
        }
        // Out-of-range input is clamped, not wrapped.
        assert_eq!(byte_from_percent(200), ANC_MAX_LEVEL);
    }

    #[test]
    fn low_battery_warns_once_below_the_threshold() {
        assert_eq!(low_battery_action(15, false), LowBattery::Notify);
        assert_eq!(low_battery_action(15, true), LowBattery::Nothing);
    }

    #[test]
    fn low_battery_rearms_only_above_twenty() {
        assert_eq!(low_battery_action(21, true), LowBattery::Rearm);
        assert_eq!(low_battery_action(20, true), LowBattery::Nothing);
        assert_eq!(low_battery_action(16, false), LowBattery::Nothing);
    }

    #[test]
    fn low_battery_ignores_zero() {
        assert_eq!(low_battery_action(0, false), LowBattery::Nothing);
        assert_eq!(low_battery_action(0, true), LowBattery::Nothing);
    }

    #[test]
    fn gauges_only_fill_for_a_real_percentage() {
        assert_eq!(lit_for(Some(BatteryLevel::Level(84))), 8.4);
        assert_eq!(lit_for(Some(BatteryLevel::InCase)), 0.0);
        assert_eq!(lit_for(Some(BatteryLevel::Unknown(0xE4))), 0.0);
        assert_eq!(lit_for(None), 0.0);
    }

    #[test]
    fn optimistic_eq_wins_for_one_second() {
        let now = Instant::now();
        let selected = EqPreset::BassBoost;
        let guard = Some((selected, now + Duration::from_secs(1)));
        assert!(eq_event_allowed(
            selected,
            guard,
            now + Duration::from_millis(500)
        ));
        assert!(!eq_event_allowed(
            EqPreset::Voice,
            guard,
            now + Duration::from_millis(999)
        ));
        assert!(eq_event_allowed(
            EqPreset::Voice,
            guard,
            now + Duration::from_secs(1)
        ));
        assert!(eq_event_allowed(EqPreset::Voice, None, now));
    }

    #[test]
    fn wakes_are_skipped_while_hidden_or_minimized() {
        assert!(should_schedule_wake(false, false));
        assert!(!should_schedule_wake(true, false));
        assert!(!should_schedule_wake(false, true));
        assert!(!should_schedule_wake(true, true));
    }
}
