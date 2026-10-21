use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Style {
    #[default]
    HiFi,
    Sonoma,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub raised_hover: Color32,
    pub track: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_tint: Color32,
    pub accent_tint_deep: Color32,
    pub on_accent: Color32,
    pub red: Color32,
    pub green: Color32,
    pub accent_sound: Color32,
}

#[derive(Clone, Copy)]
pub struct Metrics {
    pub radius: u8,
    pub radius_small: u8,
    pub title_bar: f32,
}

pub static HIFI: Palette = Palette {
    bg: Color32::from_rgb(0x11, 0x11, 0x11),
    surface: Color32::from_rgb(0x1A, 0x1A, 0x1A),
    raised: Color32::from_rgb(0x22, 0x22, 0x22),
    raised_hover: Color32::from_rgb(0x2A, 0x2A, 0x2A),
    track: Color32::from_rgb(0x2A, 0x2A, 0x2A),
    border: Color32::from_rgb(0x2E, 0x2E, 0x2E),
    text: Color32::from_rgb(0xEC, 0xE8, 0xE1),
    muted: Color32::from_rgb(0x9A, 0x95, 0x8C),
    accent: Color32::from_rgb(0xFF, 0xB0, 0x00),
    accent_hover: Color32::from_rgb(0xFF, 0xC2, 0x33),
    accent_tint: Color32::from_rgb(0x33, 0x28, 0x0F),
    accent_tint_deep: Color32::from_rgb(0x24, 0x1C, 0x0A),
    on_accent: Color32::from_rgb(0x15, 0x15, 0x15),
    red: Color32::from_rgb(0xE5, 0x69, 0x5F),
    // Hi-Fi has no green: the battery and the connected dot are amber.
    green: Color32::from_rgb(0xFF, 0xB0, 0x00),
    accent_sound: Color32::from_rgb(0xFF, 0xB0, 0x00),
};

pub static SONOMA: Palette = Palette {
    bg: Color32::from_rgb(0x1C, 0x1C, 0x1E),
    surface: Color32::from_rgb(0x2C, 0x2C, 0x2E),
    raised: Color32::from_rgb(0x3A, 0x3A, 0x3C),
    raised_hover: Color32::from_rgb(0x48, 0x48, 0x4A),
    track: Color32::from_rgb(0x3A, 0x3A, 0x3C),
    border: Color32::from_rgb(0x3A, 0x3A, 0x3C),
    text: Color32::from_rgb(0xF5, 0xF5, 0xF7),
    muted: Color32::from_rgb(0x98, 0x98, 0x9D),
    accent: Color32::from_rgb(0x0A, 0x84, 0xFF),
    accent_hover: Color32::from_rgb(0x40, 0x9C, 0xFF),
    accent_tint: Color32::from_rgb(0x12, 0x26, 0x3C),
    accent_tint_deep: Color32::from_rgb(0x12, 0x26, 0x3C),
    on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    red: Color32::from_rgb(0xFF, 0x45, 0x3A),
    green: Color32::from_rgb(0x30, 0xD1, 0x58),
    accent_sound: Color32::from_rgb(0x0A, 0x84, 0xFF),
};

pub static HIFI_METRICS: Metrics = Metrics {
    radius: 8,
    radius_small: 6,
    title_bar: 36.0,
};

pub static SONOMA_METRICS: Metrics = Metrics {
    radius: 16,
    radius_small: 10,
    title_bar: 40.0,
};

static ACTIVE: AtomicU8 = AtomicU8::new(0);

pub fn style() -> Style {
    if ACTIVE.load(Ordering::Relaxed) == 0 {
        Style::HiFi
    } else {
        Style::Sonoma
    }
}

pub fn set_style(style: Style) {
    ACTIVE.store(
        match style {
            Style::HiFi => 0,
            Style::Sonoma => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn pal() -> &'static Palette {
    match style() {
        Style::HiFi => &HIFI,
        Style::Sonoma => &SONOMA,
    }
}

pub fn metrics() -> &'static Metrics {
    match style() {
        Style::HiFi => &HIFI_METRICS,
        Style::Sonoma => &SONOMA_METRICS,
    }
}

pub fn is_sonoma() -> bool {
    style() == Style::Sonoma
}

const SPACE_REGULAR: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-Regular.ttf");
const SPACE_MEDIUM: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-Medium.ttf");
const MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf");
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const ICONS_REGULAR: &[u8] = include_bytes!("../assets/fonts/Phosphor-Regular.ttf");
const ICONS_BOLD: &[u8] = include_bytes!("../assets/fonts/Phosphor-Bold.ttf");

fn named(name: &str) -> FontFamily {
    FontFamily::Name(Arc::from(name))
}

/// UI text. Sonoma uses Inter, Hi-Fi uses Space Grotesk.
pub fn ui(size: f32) -> FontId {
    let family = if is_sonoma() {
        named("sonoma-medium")
    } else {
        named("hifi-medium")
    };
    FontId::new(size, family)
}

/// Running text, the regular weight of the active style.
pub fn body(size: f32) -> FontId {
    let family = if is_sonoma() {
        named("sonoma")
    } else {
        named("hifi")
    };
    FontId::new(size, family)
}

/// Numerals. Sonoma deliberately maps this to Inter so numbers are not
/// monospaced, while the rest of the code keeps asking for the numeral family.
pub fn mono(size: f32) -> FontId {
    let family = if is_sonoma() {
        named("sonoma-number")
    } else {
        named("hifi-number")
    };
    FontId::new(size, family)
}

pub fn mono_medium(size: f32) -> FontId {
    let family = if is_sonoma() {
        named("sonoma-number-medium")
    } else {
        named("hifi-number-medium")
    };
    FontId::new(size, family)
}

pub fn icon(size: f32) -> FontId {
    FontId::new(size, named("icons"))
}

pub fn icon_bold(size: f32) -> FontId {
    FontId::new(size, named("icons-bold"))
}

pub fn battery_color(pct: u8) -> Color32 {
    if pct <= 15 {
        pal().red
    } else {
        pal().green
    }
}

/// Registers every family once. Switching styles only changes the families the
/// helpers pick, so text never has to be rebuilt at runtime.
pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
}

pub fn apply_style(ctx: &egui::Context) {
    let palette = pal();
    let metrics = metrics();
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = visuals(palette, metrics);
    style.spacing.item_spacing = egui::vec2(10.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    // Row heights are explicit; a global 44 px minimum would inflate every
    // custom-painted row instead of just the controls that need it.
    style.spacing.interact_size.y = 0.0;
    style.text_styles = [
        (TextStyle::Heading, ui(20.0)),
        (TextStyle::Body, body(13.0)),
        (TextStyle::Button, ui(13.0)),
        (TextStyle::Small, body(11.5)),
        (TextStyle::Monospace, mono(12.0)),
    ]
    .into();
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
    ctx.set_theme(egui::ThemePreference::Dark);
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut add = |key: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(key.to_owned(), FontData::from_static(bytes).into());
    };
    add("space", SPACE_REGULAR);
    add("space-medium", SPACE_MEDIUM);
    add("mono", MONO_REGULAR);
    add("mono-medium", MONO_MEDIUM);
    add("inter", INTER_REGULAR);
    add("inter-medium", INTER_MEDIUM);
    add("icons", ICONS_REGULAR);
    add("icons-bold", ICONS_BOLD);

    // Both styles are registered from the start so switching never calls
    // set_fonts; the helpers pick the active family each frame.
    fonts
        .families
        .insert(named("hifi"), vec!["space".to_owned()]);
    fonts.families.insert(
        named("hifi-medium"),
        vec!["space-medium".to_owned(), "space".to_owned()],
    );
    fonts
        .families
        .insert(named("hifi-number"), vec!["mono".to_owned()]);
    fonts.families.insert(
        named("hifi-number-medium"),
        vec!["mono-medium".to_owned(), "mono".to_owned()],
    );
    fonts
        .families
        .insert(named("sonoma"), vec!["inter".to_owned()]);
    fonts.families.insert(
        named("sonoma-medium"),
        vec!["inter-medium".to_owned(), "inter".to_owned()],
    );
    fonts
        .families
        .insert(named("sonoma-number"), vec!["inter".to_owned()]);
    fonts.families.insert(
        named("sonoma-number-medium"),
        vec!["inter-medium".to_owned(), "inter".to_owned()],
    );
    fonts
        .families
        .insert(named("icons"), vec!["icons".to_owned()]);
    fonts
        .families
        .insert(named("icons-bold"), vec!["icons-bold".to_owned()]);
    fonts
}

fn visuals(palette: &Palette, metrics: &Metrics) -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = palette.bg;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = CornerRadius::same(metrics.radius);
    visuals.extreme_bg_color = palette.bg;
    visuals.faint_bg_color = palette.surface;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent_tint;
    visuals.selection.stroke = Stroke::new(1.5, palette.accent);
    visuals.slider_trailing_fill = true;

    let base = egui::style::WidgetVisuals {
        bg_fill: palette.raised,
        weak_bg_fill: palette.raised,
        bg_stroke: Stroke::new(1.0, palette.border),
        corner_radius: CornerRadius::same(metrics.radius_small),
        fg_stroke: Stroke::new(1.0, palette.text),
        expansion: 0.0,
    };
    visuals.widgets.noninteractive = egui::style::WidgetVisuals {
        bg_fill: palette.surface,
        weak_bg_fill: palette.surface,
        ..base
    };
    visuals.widgets.inactive = base;
    visuals.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: palette.raised_hover,
        weak_bg_fill: palette.raised_hover,
        bg_stroke: Stroke::new(1.0, palette.accent_tint),
        ..base
    };
    visuals.widgets.active = egui::style::WidgetVisuals {
        bg_fill: palette.accent_tint,
        weak_bg_fill: palette.accent_tint,
        bg_stroke: Stroke::new(1.0, palette.accent),
        ..base
    };
    visuals.widgets.open = egui::style::WidgetVisuals {
        bg_fill: palette.accent_tint,
        weak_bg_fill: palette.accent_tint,
        bg_stroke: Stroke::new(1.0, palette.accent),
        ..base
    };

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: Color32) -> f32 {
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (high, low) = {
            let (la, lb) = (luminance(a), luminance(b));
            if la >= lb {
                (la, lb)
            } else {
                (lb, la)
            }
        };
        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn palettes_keep_text_readable() {
        for palette in [&HIFI, &SONOMA] {
            assert!(
                contrast(palette.text, palette.bg) >= 4.5,
                "texto/fondo en {:?}",
                palette.bg
            );
            assert!(
                contrast(palette.text, palette.surface) >= 4.5,
                "texto/tarjeta en {:?}",
                palette.surface
            );
            assert!(
                contrast(palette.muted, palette.bg) >= 3.0,
                "secundario/fondo en {:?}",
                palette.bg
            );
            assert!(
                contrast(palette.muted, palette.surface) >= 3.0,
                "secundario/tarjeta en {:?}",
                palette.surface
            );
        }
    }
}
