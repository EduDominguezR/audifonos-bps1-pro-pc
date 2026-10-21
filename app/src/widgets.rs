use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    TextureHandle, Ui, Vec2, WidgetInfo,
};

use crate::theme;

pub const SEGMENTS: usize = 10;

/// A rounded surface that groups one idea. Returns whatever the closure returns.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::default()
        .fill(theme::pal().surface)
        .stroke(Stroke::new(1.0, theme::pal().border))
        .corner_radius(CornerRadius::same(theme::metrics().radius))
        .inner_margin(egui::Margin::same(if theme::is_sonoma() { 6 } else { 4 }))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Uppercase label with the slight tracking of a hardware faceplate.
pub fn caps(ui: &Ui, pos: Pos2, text: &str, size: f32, color: Color32, align: Align2) {
    let mut job = LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        TextFormat {
            font_id: theme::ui(size),
            color,
            extra_letter_spacing: 0.9,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let rect = align.anchor_size(pos, galley.size());
    ui.painter().galley(rect.min, galley, color);
}

fn caps_height(size: f32) -> f32 {
    (size * 1.27).ceil()
}

/// The small heading that separates blocks, optionally with a right-hand note.
pub fn section(ui: &mut Ui, text: &str, meta: Option<&str>) {
    let height = caps_height(11.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    caps(
        ui,
        rect.left_center(),
        &text.to_uppercase(),
        11.0,
        theme::pal().muted,
        Align2::LEFT_CENTER,
    );
    if let Some(meta) = meta {
        ui.painter().text(
            rect.right_center(),
            Align2::RIGHT_CENTER,
            meta,
            theme::mono(12.0),
            theme::pal().muted,
        );
    }
}

/// A segmented control. `pill` is the sliding highlight position in option
/// units; callers that do not animate can pass the selected index.
pub fn segmented(
    ui: &mut Ui,
    id: egui::Id,
    options: &[&str],
    selected: Option<usize>,
    pill: f32,
    size: Vec2,
) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let sonoma = theme::is_sonoma();
    let radius = if sonoma { 9 } else { 6 };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(radius),
        if sonoma {
            theme::pal().track
        } else {
            theme::pal().raised
        },
    );
    if !sonoma {
        painter.rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(1.0, theme::pal().border),
            StrokeKind::Inside,
        );
    }

    let pad = 3.0;
    let count = options.len().max(1) as f32;
    let option_width = (rect.width() - pad * 2.0) / count;
    if selected.is_some() {
        let position = pill.clamp(0.0, count - 1.0);
        let pill_rect = Rect::from_min_size(
            egui::pos2(
                rect.left() + pad + option_width * position,
                rect.top() + pad,
            ),
            Vec2::new(option_width, rect.height() - pad * 2.0),
        );
        if sonoma {
            painter.rect_filled(
                pill_rect.translate(Vec2::new(0.0, 1.0)),
                CornerRadius::same(radius - 2),
                Color32::from_black_alpha(45),
            );
            painter.rect_filled(
                pill_rect,
                CornerRadius::same(radius - 2),
                theme::pal().raised_hover,
            );
        } else {
            painter.rect_filled(
                pill_rect,
                CornerRadius::same(radius - 2),
                theme::pal().accent_tint,
            );
            painter.rect_stroke(
                pill_rect,
                CornerRadius::same(radius - 2),
                Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.7)),
                StrokeKind::Inside,
            );
        }
    }

    let mut chosen = None;
    let mut focused = false;
    for (index, label) in options.iter().enumerate() {
        let option_rect = Rect::from_min_size(
            egui::pos2(
                rect.left() + pad + option_width * index as f32,
                rect.top() + pad,
            ),
            Vec2::new(option_width, rect.height() - pad * 2.0),
        );
        let response = ui.interact(option_rect, id.with(index), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(
                egui::Role::RadioButton,
                true,
                selected == Some(index),
                *label,
            )
        });
        focused |= response.has_focus();
        let active = selected == Some(index);
        let color = if active {
            if sonoma {
                theme::pal().text
            } else {
                theme::pal().accent
            }
        } else if response.hovered() {
            theme::pal().text
        } else {
            theme::pal().muted
        };
        painter.text(
            option_rect.center(),
            Align2::CENTER_CENTER,
            *label,
            if active {
                theme::ui(12.5)
            } else {
                theme::body(12.5)
            },
            color,
        );
        if activated(ui, &response) {
            chosen = Some(index);
        }
    }
    if focused {
        painter.rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(2.0, theme::pal().accent),
            StrokeKind::Inside,
        );
    }
    chosen
}

/// A 2px accent outline, drawn outside the widget so it never shifts layout.
pub fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(radius.saturating_add(2)),
            Stroke::new(2.0, theme::pal().accent),
            StrokeKind::Outside,
        );
    }
}

fn activated(ui: &Ui, response: &Response) -> bool {
    if response.clicked() {
        return true;
    }
    response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        })
}

/// An equalizer preset card: icon, label, hint and a faint response curve.
/// The caller animates the width; this only paints.
pub fn eq_choice(
    ui: &mut Ui,
    title: &str,
    hint: &str,
    icon: &str,
    curve: &[f32; 8],
    selected: bool,
    size: Vec2,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, true, selected, title));

    if theme::is_sonoma() {
        let hovered = response.hovered();
        let fill = if selected {
            theme::pal().accent_tint
        } else if hovered {
            theme::pal().raised_hover
        } else {
            theme::pal().raised
        };
        let border = if selected {
            theme::pal().accent.gamma_multiply(0.85)
        } else {
            theme::pal().border
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
            StrokeKind::Inside,
        );

        let left = rect.left() + 9.0;
        let right = rect.right() - 9.0;
        let base = rect.bottom() - 4.0;
        let points: Vec<Pos2> = curve
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let x = left + (right - left) * index as f32 / (curve.len() - 1) as f32;
                Pos2::new(x, base - value.clamp(0.0, 1.0) * 5.0)
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(
                1.0,
                if selected {
                    theme::pal().accent.gamma_multiply(0.30)
                } else {
                    theme::pal().muted.gamma_multiply(0.16)
                },
            ),
        ));

        let box_rect = Rect::from_center_size(
            egui::pos2(rect.left() + 22.0, rect.center().y),
            Vec2::splat(24.0),
        );
        painter.rect_filled(
            box_rect,
            CornerRadius::same(7),
            if selected {
                theme::pal().accent.gamma_multiply(0.18)
            } else {
                theme::pal().track
            },
        );
        painter.text(
            box_rect.center(),
            Align2::CENTER_CENTER,
            icon,
            theme::icon(14.0),
            if selected {
                theme::pal().accent
            } else {
                theme::pal().muted
            },
        );
        painter.text(
            egui::pos2(rect.left() + 40.0, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            theme::ui(12.5),
            theme::pal().text,
        );
        if selected {
            painter.text(
                egui::pos2(rect.right() - 9.0, rect.center().y),
                Align2::RIGHT_CENTER,
                crate::icons::CHECK,
                theme::icon_bold(13.0),
                theme::pal().accent,
            );
        }
        focus_ring(ui, &response, rect, theme::metrics().radius_small);
        return activated(ui, &response);
    }

    let hovered = response.hovered();
    let fill = if selected {
        theme::pal().accent_tint_deep
    } else if hovered {
        theme::pal().raised_hover
    } else {
        theme::pal().raised
    };
    let border = if selected {
        theme::pal().accent
    } else {
        theme::pal().border
    };
    let painter = ui.painter();
    if selected {
        painter.rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(theme::metrics().radius + 1),
            Stroke::new(4.0, theme::pal().accent.gamma_multiply(0.10)),
            StrokeKind::Outside,
        );
    }
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::metrics().radius_small),
        fill,
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::metrics().radius_small),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );

    let curve_color = if selected {
        theme::pal().accent.gamma_multiply(0.45)
    } else {
        theme::pal().muted.gamma_multiply(0.28)
    };
    let left = rect.left() + 10.0;
    let right = rect.right() - 10.0;
    let base = rect.bottom() - 3.0;
    let points: Vec<Pos2> = curve
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = left + (right - left) * index as f32 / (curve.len() - 1) as f32;
            Pos2::new(x, base - value.clamp(0.0, 1.0) * 6.0)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.0, curve_color)));

    painter.text(
        Pos2::new(left, rect.top() + 6.0),
        Align2::LEFT_TOP,
        title,
        theme::ui(12.5),
        theme::pal().text,
    );
    painter.text(
        Pos2::new(left, rect.top() + 20.0),
        Align2::LEFT_TOP,
        hint,
        theme::body(12.0),
        theme::pal().muted,
    );
    painter.text(
        Pos2::new(rect.right() - 10.0, rect.top() + 7.0),
        Align2::RIGHT_TOP,
        icon,
        theme::icon(15.0),
        if selected {
            theme::pal().accent
        } else {
            theme::pal().muted
        },
    );

    focus_ring(ui, &response, rect, theme::metrics().radius_small);
    activated(ui, &response)
}

/// A full-width setting row: label left, optional hint and an industrial switch
/// on the right. Returns `Some(new_value)` when the user changes it.
pub fn toggle_row(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    on: bool,
    hint: Option<&str>,
) -> Option<bool> {
    let row_height = if theme::is_sonoma() { 34.0 } else { 44.0 };
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, true, on, label));

    if theme::is_sonoma() {
        let painter = ui.painter();
        painter.text(
            egui::pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::ui(13.0),
            theme::pal().text,
        );
        if let Some(hint) = hint {
            painter.text(
                egui::pos2(rect.right() - 56.0, rect.center().y),
                Align2::RIGHT_CENTER,
                hint,
                theme::body(12.0),
                theme::pal().muted,
            );
        }
        let switch = Rect::from_min_size(
            egui::pos2(rect.right() - 44.0, rect.center().y - 13.0),
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
            StrokeKind::Inside,
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
        focus_ring(ui, &response, switch, 13);
        return if activated(ui, &response) {
            Some(!on)
        } else {
            None
        };
    }

    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::ui(12.5),
        theme::pal().text,
    );

    let switch = Rect::from_min_size(
        egui::pos2(rect.right() - 44.0, rect.center().y - 12.0),
        Vec2::new(44.0, 24.0),
    );
    if let Some(hint) = hint {
        painter.text(
            egui::pos2(switch.left() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            theme::body(12.0),
            theme::pal().muted,
        );
    }

    let track = if on {
        theme::pal().accent
    } else {
        theme::pal().track
    };
    painter.rect_filled(switch, CornerRadius::same(4), track);
    painter.rect_stroke(
        switch,
        CornerRadius::same(4),
        Stroke::new(
            1.0,
            if on {
                theme::pal().accent_hover
            } else {
                theme::pal().border
            },
        ),
        StrokeKind::Inside,
    );

    // The I/O mark stays visible at low contrast, like a real rocker switch.
    let ink = if on {
        theme::pal().on_accent
    } else {
        theme::pal().muted
    };
    painter.text(
        egui::pos2(switch.left() + 7.0, switch.center().y),
        Align2::CENTER_CENTER,
        "I",
        theme::mono_medium(7.5),
        ink.gamma_multiply(0.8),
    );
    painter.text(
        egui::pos2(switch.right() - 7.0, switch.center().y),
        Align2::CENTER_CENTER,
        "O",
        theme::mono_medium(7.5),
        ink.gamma_multiply(0.8),
    );

    let lever = Rect::from_center_size(
        egui::pos2(
            if on {
                switch.right() - 12.0
            } else {
                switch.left() + 12.0
            },
            switch.center().y,
        ),
        Vec2::new(18.0, 16.0),
    );
    painter.rect_filled(
        lever,
        CornerRadius::same(2),
        if on {
            theme::pal().on_accent
        } else {
            theme::pal().muted
        },
    );

    focus_ring(ui, &response, rect, 8);
    if activated(ui, &response) {
        Some(!on)
    } else {
        None
    }
}

/// One battery readout: name, big mono number and a ten-segment gauge.
///
/// `lit` is the animated number of lit segments, so the caller owns timing.
#[allow(clippy::too_many_arguments)]
pub fn battery_tile(
    ui: &mut Ui,
    width: f32,
    label: &str,
    level: Option<baseus_protocol::BatteryLevel>,
    charging: bool,
    is_bud: bool,
    lit: f32,
) {
    use baseus_protocol::BatteryLevel;

    if theme::is_sonoma() {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 60.0), Sense::hover());
        let painter = ui.painter();
        let radius = 14;
        painter.rect_filled(rect, CornerRadius::same(radius), theme::pal().surface);
        painter.rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(1.0, theme::pal().border),
            StrokeKind::Inside,
        );
        painter.text(
            egui::pos2(rect.left() + 9.0, rect.top() + 7.0),
            Align2::LEFT_TOP,
            label,
            theme::ui(11.5),
            theme::pal().muted,
        );
        let percent = level.and_then(|level| level.percent());
        let accent = percent
            .map(theme::battery_color)
            .unwrap_or(theme::pal().muted);
        let icon = if charging {
            crate::icons::BATTERY_CHARGING
        } else {
            crate::icons::BATTERY_HIGH
        };
        painter.text(
            egui::pos2(rect.right() - 9.0, rect.top() + 7.0),
            Align2::RIGHT_TOP,
            icon,
            theme::icon(14.0),
            if charging { theme::pal().green } else { accent },
        );

        let value_top = rect.top() + 21.0;
        match level {
            Some(BatteryLevel::Level(value)) => {
                painter.text(
                    egui::pos2(rect.left() + 9.0, value_top),
                    Align2::LEFT_TOP,
                    format!("{value}%"),
                    theme::ui(23.0),
                    theme::battery_color(value),
                );
            }
            Some(BatteryLevel::InCase) if is_bud => {
                painter.text(
                    egui::pos2(rect.left() + 9.0, value_top + 6.0),
                    Align2::LEFT_TOP,
                    "En estuche",
                    theme::ui(12.0),
                    theme::pal().muted,
                );
            }
            _ => {
                painter.text(
                    egui::pos2(rect.left() + 9.0, value_top),
                    Align2::LEFT_TOP,
                    "—",
                    theme::ui(23.0),
                    theme::pal().muted,
                );
            }
        }

        let bar = Rect::from_min_size(
            egui::pos2(rect.left() + 9.0, rect.bottom() - 9.0),
            Vec2::new(rect.width() - 18.0, 4.0),
        );
        painter.rect_filled(bar, CornerRadius::same(2), theme::pal().track);
        if let Some(value) = percent {
            let filled = Rect::from_min_size(
                bar.min,
                Vec2::new(bar.width() * f32::from(value) / 100.0, bar.height()),
            );
            painter.rect_filled(filled, CornerRadius::same(2), theme::battery_color(value));
        }
        return;
    }

    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 52.0), Sense::hover());
    let painter = ui.painter();
    caps(
        ui,
        rect.left_top(),
        label,
        11.0,
        theme::pal().muted,
        Align2::LEFT_TOP,
    );

    let value_top = rect.top() + 13.0;
    match level {
        Some(BatteryLevel::Level(percent)) => {
            let color = theme::battery_color(percent);
            painter.text(
                egui::pos2(rect.left(), value_top),
                Align2::LEFT_TOP,
                format!("{percent}%"),
                theme::mono_medium(28.0),
                color,
            );
        }
        Some(BatteryLevel::InCase) if is_bud => {
            caps(
                ui,
                egui::pos2(rect.left(), value_top + 8.0),
                "En estuche",
                11.0,
                theme::pal().muted,
                Align2::LEFT_TOP,
            );
        }
        _ => {
            painter.text(
                egui::pos2(rect.left(), value_top),
                Align2::LEFT_TOP,
                "—",
                theme::mono_medium(28.0),
                theme::pal().muted,
            );
        }
    }

    if charging {
        painter.text(
            egui::pos2(rect.right(), value_top + 4.0),
            Align2::RIGHT_TOP,
            crate::icons::LIGHTNING,
            theme::icon_bold(13.0),
            theme::pal().accent,
        );
    }

    let percent = level.and_then(|level| level.percent());
    let color = percent
        .map(theme::battery_color)
        .unwrap_or(theme::pal().accent);
    let count = lit.clamp(0.0, SEGMENTS as f32).round() as usize;
    let gap = 2.0;
    let segment_width = (rect.width() - gap * (SEGMENTS as f32 - 1.0)) / SEGMENTS as f32;
    let y = rect.bottom() - 7.0;
    for index in 0..SEGMENTS {
        let x = rect.left() + index as f32 * (segment_width + gap);
        let segment = Rect::from_min_size(egui::pos2(x, y), Vec2::new(segment_width, 7.0));
        let on = index < count;
        painter.rect_filled(
            segment,
            CornerRadius::same(1),
            if on {
                color
            } else {
                theme::pal().track.gamma_multiply(0.45)
            },
        );
    }
}

/// Mixer-style strength slider: thin track, eleven ticks and a square knob.
/// The caller compares the value before and after to detect a change.
pub fn level_slider(ui: &mut Ui, percent: &mut u8, enabled: bool) -> Response {
    let desired = Vec2::new(ui.available_width(), 24.0);
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let id = response.id;
    if response.clicked() || response.drag_started() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }

    let mut value = f32::from(*percent) / 100.0;
    if enabled {
        if response.dragged() || response.clicked() {
            if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                let track = track(rect);
                value = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
                response.mark_changed();
            }
        }
        if response.has_focus() {
            let step = ui.input(|input| {
                if input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::ArrowUp)
                {
                    Some(0.05)
                } else if input.key_pressed(egui::Key::ArrowLeft)
                    || input.key_pressed(egui::Key::ArrowDown)
                {
                    Some(-0.05)
                } else if input.key_pressed(egui::Key::Home) {
                    Some(-1.0)
                } else if input.key_pressed(egui::Key::End) {
                    Some(1.0)
                } else {
                    None
                }
            });
            if let Some(step) = step {
                value = (value + step).clamp(0.0, 1.0);
                response.mark_changed();
            }
        }
    }
    *percent = (value * 100.0).round() as u8;

    response.widget_info(|| {
        WidgetInfo::labeled(
            egui::Role::Slider,
            enabled,
            format!("Intensidad {}%", *percent),
        )
    });

    let track = track(rect);
    if theme::is_sonoma() {
        let painter = ui.painter();
        let color = if enabled {
            theme::pal().accent
        } else {
            theme::pal().muted.gamma_multiply(0.40)
        };
        painter.rect_filled(track, CornerRadius::same(2), theme::pal().track);
        let filled = Rect::from_min_size(track.min, Vec2::new(track.width() * value, 4.0));
        painter.rect_filled(filled, CornerRadius::same(2), color);
        let knob_pos = Pos2::new(track.left() + track.width() * value, track.center().y);
        painter.circle_filled(
            knob_pos + Vec2::new(0.0, 1.0),
            10.5,
            Color32::from_black_alpha(55),
        );
        painter.circle_filled(knob_pos, 9.5, Color32::WHITE);
        focus_ring(
            ui,
            &response,
            Rect::from_center_size(track.center(), Vec2::new(track.width(), 22.0)),
            11,
        );
        if !enabled {
            response = response.on_disabled_hover_text("Solo con cancelación activa");
        }
        return response;
    }
    let painter = ui.painter();
    let glow = enabled && (response.dragged() || response.hovered());
    let color = if enabled {
        theme::pal().accent
    } else {
        theme::pal().muted.gamma_multiply(0.45)
    };
    painter.rect_filled(
        Rect::from_center_size(track.center(), Vec2::new(track.width(), 4.0)),
        CornerRadius::same(2),
        theme::pal().track,
    );
    let filled = Rect::from_min_size(track.min, Vec2::new(track.width() * value, 4.0));
    painter.rect_filled(filled, CornerRadius::same(2), color);

    // Eleven mixer ticks.
    for index in 0..=10 {
        let x = track.left() + track.width() * index as f32 / 10.0;
        let tall = index % 5 == 0;
        painter.line_segment(
            [
                Pos2::new(x, track.center().y - if tall { 7.0 } else { 4.0 }),
                Pos2::new(x, track.center().y + if tall { 7.0 } else { 4.0 }),
            ],
            Stroke::new(
                1.0,
                theme::pal()
                    .muted
                    .gamma_multiply(if enabled { 0.55 } else { 0.3 }),
            ),
        );
    }

    let knob_pos = Pos2::new(track.left() + track.width() * value, track.center().y);
    if glow {
        painter.circle_filled(knob_pos, 14.0, theme::pal().accent.gamma_multiply(0.16));
    }
    let knob = Rect::from_center_size(knob_pos, Vec2::new(16.0, 16.0));
    painter.rect_filled(knob, CornerRadius::same(3), color);
    painter.rect_stroke(
        knob,
        CornerRadius::same(3),
        Stroke::new(1.0, theme::pal().bg.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );

    if !enabled {
        response = response.on_disabled_hover_text("Solo con cancelación activa");
    }
    focus_ring(
        ui,
        &response,
        Rect::from_center_size(track.center(), Vec2::new(track.width(), 20.0)),
        6,
    );
    response
}

fn track(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.left() + 6.0, rect.center().y),
        Pos2::new(rect.right() - 6.0, rect.center().y),
    )
}

/// Visual weight of a [`wide_button`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one action the screen is asking for.
    Primary,
    /// Everything else.
    Secondary,
    /// A secondary action that is currently active, like a running tone.
    Tonal,
}

/// A wide action button with centered text, a focus ring and keyboard
/// activation.
///
/// Hand-rolled because egui's `Button` keeps its label at the left edge when
/// `min_size` is wider than the text.
pub fn wide_button(ui: &mut Ui, label: &str, size: Vec2, kind: Kind) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));

    let hovered = response.hovered();
    if theme::is_sonoma() {
        let (fill, border, text) = match kind {
            Kind::Primary => (
                if hovered {
                    theme::pal().accent_hover
                } else {
                    theme::pal().accent
                },
                Color32::TRANSPARENT,
                theme::pal().on_accent,
            ),
            Kind::Secondary => (
                if hovered {
                    theme::pal().raised_hover
                } else {
                    theme::pal().raised
                },
                Color32::TRANSPARENT,
                theme::pal().text,
            ),
            Kind::Tonal => (
                theme::pal().accent_tint,
                theme::pal().accent.gamma_multiply(0.6),
                theme::pal().accent,
            ),
        };
        let painter = ui.painter();
        let radius = 12;
        painter.rect_filled(rect, CornerRadius::same(radius), fill);
        if border != Color32::TRANSPARENT {
            painter.rect_stroke(
                rect,
                CornerRadius::same(radius),
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
        }
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            theme::ui(13.0),
            text,
        );
        focus_ring(ui, &response, rect, radius);
        return activated(ui, &response);
    }
    let (fill, border, text) = match kind {
        Kind::Primary => (
            if hovered {
                theme::pal().accent_hover
            } else {
                theme::pal().accent
            },
            theme::pal().accent,
            theme::pal().on_accent,
        ),
        Kind::Secondary => (
            if hovered {
                theme::pal().raised_hover
            } else {
                theme::pal().raised
            },
            if hovered {
                theme::pal().accent
            } else {
                theme::pal().border
            },
            theme::pal().text,
        ),
        Kind::Tonal => (
            theme::pal().accent_tint,
            theme::pal().accent,
            theme::pal().accent,
        ),
    };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme::metrics().radius), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::metrics().radius),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::ui(13.0),
        text,
    );

    focus_ring(ui, &response, rect, theme::metrics().radius);
    activated(ui, &response)
}

/// One 128 px tile of fixed-seed grain, tiled over everything behind the cards.
pub fn grain(ctx: &egui::Context) -> TextureHandle {
    let mut pixels = Vec::with_capacity(128 * 128);
    let mut state = 0x9E37_79B9u32;
    for _ in 0..128 * 128 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let light = state & 0x8000_0000 != 0;
        if light {
            pixels.push(Color32::from_rgba_unmultiplied(255, 255, 255, 4));
        } else {
            pixels.push(Color32::from_rgba_unmultiplied(0, 0, 0, 4));
        }
    }
    let image = egui::ColorImage::new([128, 128], pixels);
    ctx.load_texture(
        "grain",
        image,
        egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..Default::default()
        },
    )
}

pub fn paint_grain(ui: &Ui, texture: &TextureHandle, rect: Rect) {
    let uv = Rect::from_min_size(Pos2::ZERO, rect.size() / 128.0);
    ui.painter()
        .add(egui::Shape::image(texture.id(), rect, uv, Color32::WHITE));
}
