//! Colours, fonts and small drawing helpers. Light and dark palettes; dark account rail in both.
//! Font: Manrope (SIL OFL 1.1, bundled in `assets/fonts`, see ATTRIBUTION.md), the same face
//! JupiterOS uses.

use egui::{Color32, FontFamily, RichText};
use mailopen_core::ThemePref;

pub const ERROR: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
pub const OK: Color32 = Color32::from_rgb(0x16, 0xA3, 0x4A);

/// A colour style the user can pick: accent (selection, badges, buttons) and the account rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub name: &'static str,
    pub accent: Color32,
    pub rail_light: Color32,
    pub rail_dark: Color32,
}

pub const STYLES: [Style; 8] = [
    Style {
        name: "Blu",
        accent: Color32::from_rgb(0x2F, 0x7B, 0xF5),
        rail_light: Color32::from_rgb(0x1B, 0x3B, 0x63),
        rail_dark: Color32::from_rgb(0x12, 0x1A, 0x27),
    },
    Style {
        name: "Verde",
        accent: Color32::from_rgb(0x16, 0xA3, 0x4A),
        rail_light: Color32::from_rgb(0x14, 0x3D, 0x2B),
        rail_dark: Color32::from_rgb(0x0F, 0x1F, 0x18),
    },
    Style {
        name: "Viola",
        accent: Color32::from_rgb(0x7C, 0x3A, 0xED),
        rail_light: Color32::from_rgb(0x2E, 0x1F, 0x5A),
        rail_dark: Color32::from_rgb(0x19, 0x13, 0x2B),
    },
    Style {
        name: "Arancio",
        accent: Color32::from_rgb(0xF9, 0x73, 0x16),
        rail_light: Color32::from_rgb(0x4A, 0x2A, 0x14),
        rail_dark: Color32::from_rgb(0x24, 0x18, 0x10),
    },
    Style {
        name: "Rosso",
        accent: Color32::from_rgb(0xDC, 0x26, 0x26),
        rail_light: Color32::from_rgb(0x4A, 0x1C, 0x1C),
        rail_dark: Color32::from_rgb(0x26, 0x12, 0x12),
    },
    Style {
        name: "Ciano",
        accent: Color32::from_rgb(0x08, 0x91, 0xB2),
        rail_light: Color32::from_rgb(0x0F, 0x37, 0x44),
        rail_dark: Color32::from_rgb(0x0C, 0x1E, 0x25),
    },
    Style {
        name: "Rosa",
        accent: Color32::from_rgb(0xDB, 0x27, 0x77),
        rail_light: Color32::from_rgb(0x4A, 0x16, 0x30),
        rail_dark: Color32::from_rgb(0x27, 0x10, 0x1C),
    },
    Style {
        name: "Grafite",
        accent: Color32::from_rgb(0x4B, 0x55, 0x63),
        rail_light: Color32::from_rgb(0x1F, 0x29, 0x37),
        rail_dark: Color32::from_rgb(0x14, 0x17, 0x1D),
    },
];

pub fn style_by_name(name: &str) -> Style {
    STYLES
        .iter()
        .find(|s| s.name == name)
        .copied()
        .unwrap_or(STYLES[0])
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

const STYLE_KEY: &str = "mailopen-style-name";

/// Make `name` the active style for this context (cheap when unchanged).
pub fn apply_style(ctx: &egui::Context, name: &str) {
    let id = egui::Id::unique(STYLE_KEY);
    let current: Option<String> = ctx.data(|d| d.get_temp(id));
    if current.as_deref() == Some(name) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, name.to_string()));
    let s = style_by_name(name);
    ctx.set_visuals_of(egui::Theme::Light, visuals(false, s));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true, s));
}

fn current_style(ctx: &egui::Context) -> Style {
    let id = egui::Id::unique(STYLE_KEY);
    let name: Option<String> = ctx.data(|d| d.get_temp(id));
    style_by_name(name.as_deref().unwrap_or(""))
}

/// Account / avatar colours. Index = `Account::color`.
pub const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x3B, 0x82, 0xF6),
    Color32::from_rgb(0x10, 0xB9, 0x81),
    Color32::from_rgb(0xF5, 0x9E, 0x0B),
    Color32::from_rgb(0xEF, 0x44, 0x44),
    Color32::from_rgb(0x8B, 0x5C, 0xF6),
    Color32::from_rgb(0xEC, 0x48, 0x99),
    Color32::from_rgb(0x14, 0xB8, 0xA6),
    Color32::from_rgb(0x64, 0x74, 0x8B),
];

/// The colours that differ between light and dark.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub accent: Color32,
    pub rail_bg: Color32,
    pub rail_selected: Color32,
    pub accent_soft: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub hover: Color32,
}

pub const LIGHT: Tokens = Tokens {
    accent: Color32::from_rgb(0x2F, 0x7B, 0xF5),
    rail_bg: Color32::from_rgb(0x1B, 0x3B, 0x63),
    rail_selected: Color32::from_rgb(0x2B, 0x54, 0x85),
    accent_soft: Color32::from_rgb(0xE8, 0xF0, 0xFE),
    text: Color32::from_rgb(0x1F, 0x29, 0x37),
    muted: Color32::from_rgb(0x6B, 0x72, 0x80),
    border: Color32::from_rgb(0xE5, 0xE7, 0xEB),
    surface: Color32::WHITE,
    surface_alt: Color32::from_rgb(0xF5, 0xF6, 0xF8),
    hover: Color32::from_rgb(0xF1, 0xF3, 0xF6),
};

pub const DARK: Tokens = Tokens {
    accent: Color32::from_rgb(0x2F, 0x7B, 0xF5),
    rail_bg: Color32::from_rgb(0x12, 0x1A, 0x27),
    rail_selected: Color32::from_rgb(0x24, 0x33, 0x4C),
    accent_soft: Color32::from_rgb(0x1F, 0x2F, 0x4A),
    text: Color32::from_rgb(0xE6, 0xE8, 0xEC),
    muted: Color32::from_rgb(0x9A, 0xA3, 0xB2),
    border: Color32::from_rgb(0x2F, 0x33, 0x3B),
    surface: Color32::from_rgb(0x1C, 0x1E, 0x24),
    surface_alt: Color32::from_rgb(0x25, 0x28, 0x30),
    hover: Color32::from_rgb(0x2A, 0x2D, 0x36),
};

pub fn tokens(ui: &egui::Ui) -> Tokens {
    let dark = ui.visuals().dark_mode;
    styled_tokens(dark, current_style(ui.ctx()))
}

fn styled_tokens(dark: bool, s: Style) -> Tokens {
    let mut t = if dark { DARK } else { LIGHT };
    t.accent = s.accent;
    t.rail_bg = if dark { s.rail_dark } else { s.rail_light };
    t.rail_selected = mix(t.rail_bg, Color32::WHITE, 0.14);
    t.accent_soft = if dark {
        mix(t.surface, s.accent, 0.28)
    } else {
        mix(Color32::WHITE, s.accent, 0.12)
    };
    t
}

pub fn account_color(index: u8) -> Color32 {
    PALETTE
        .get(usize::from(index) % PALETTE.len())
        .copied()
        .unwrap_or(Color32::GRAY)
}

/// A stable colour for any string (sender address, folder name).
pub fn hash_color(s: &str) -> Color32 {
    let h = s
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    PALETTE
        .get((h % PALETTE.len() as u32) as usize)
        .copied()
        .unwrap_or(Color32::GRAY)
}

const MANROPE_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Manrope-Regular.ttf");
const MANROPE_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Manrope-SemiBold.ttf");

/// Install fonts and both palettes once per context. Returns false on the frame that installs
/// them (fonts become available on the next frame), so the caller skips drawing that frame.
pub fn ensure(ctx: &egui::Context, pref: ThemePref) -> bool {
    let id = egui::Id::unique("mailopen-theme-installed");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return true;
    }
    install(ctx);
    apply_pref(ctx, pref);
    ctx.data_mut(|d| d.insert_temp(id, true));
    false
}

pub fn apply_pref(ctx: &egui::Context, pref: ThemePref) {
    let p = match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(p);
}

fn visuals(dark: bool, s: Style) -> egui::Visuals {
    let t = styled_tokens(dark, s);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = t.surface;
    v.window_fill = t.surface;
    v.extreme_bg_color = t.surface_alt;
    v.faint_bg_color = t.hover;
    v.override_text_color = Some(t.text);
    v.selection.bg_fill = t.accent;
    v.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, t.border);
    v.widgets.inactive.bg_fill = t.surface_alt;
    v.widgets.inactive.weak_bg_fill = t.surface_alt;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.active.bg_fill = t.accent_soft;
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 40 }),
    };
    v.window_corner_radius = egui::CornerRadius::same(10);
    v.window_stroke = egui::Stroke::new(1.0, t.border);
    v
}

/// Bundled Manrope (regular + semibold) and both palettes.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(false, STYLES[0]));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true, STYLES[0]));
    ctx.add_font(egui::epaint::text::FontInsert {
        name: "manrope".into(),
        data: egui::FontData::from_static(MANROPE_REGULAR),
        families: vec![egui::epaint::text::InsertFontFamily {
            family: FontFamily::Proportional,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    });
    ctx.add_font(egui::epaint::text::FontInsert {
        name: "manrope-semibold".into(),
        data: egui::FontData::from_static(MANROPE_SEMIBOLD),
        families: vec![egui::epaint::text::InsertFontFamily {
            family: FontFamily::Name("bold".into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    });
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size = egui::vec2(40.0, 30.0);
        use egui::{FontId, TextStyle};
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(20.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.5));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.0));
    });
}

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn bold_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, bold_family())
}

/// Semibold text in the current text colour.
pub fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(bold_family()).size(size)
}

/// A coloured circle with one letter inside (an avatar), or the sender's logo when known.
pub fn avatar(
    ui: &mut egui::Ui,
    letter: &str,
    color: Color32,
    radius: f32,
    logo: Option<egui::load::SizedTexture>,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(radius * 2.0, radius * 2.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        draw_avatar(ui, rect, letter, color, logo);
    }
    resp
}

/// Draw an avatar into `rect`: the logo clipped to a circle, else the lettered disc.
pub fn draw_avatar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    letter: &str,
    color: Color32,
    logo: Option<egui::load::SizedTexture>,
) {
    match logo {
        Some(tex) => {
            let r = rect.width().min(rect.height()) / 2.0;
            paint_texture(ui.painter(), rect, tex.id, r);
        }
        None => paint_avatar(
            ui.painter(),
            rect.center(),
            letter,
            color,
            rect.width() / 2.0,
        ),
    }
}

pub fn paint_avatar(
    p: &egui::Painter,
    center: egui::Pos2,
    letter: &str,
    color: Color32,
    radius: f32,
) {
    p.circle_filled(center, radius, color);
    p.text(
        center,
        egui::Align2::CENTER_CENTER,
        letter,
        bold_font(radius * 1.05),
        Color32::WHITE,
    );
}

/// A texture in a rounded rectangle, drawn directly (no layout side effects).
pub fn paint_texture(p: &egui::Painter, rect: egui::Rect, id: egui::TextureId, radius: f32) {
    let shape = egui::epaint::RectShape::filled(
        rect,
        egui::CornerRadius::same(radius.clamp(0.0, 255.0) as u8),
        Color32::WHITE,
    )
    .with_texture(
        id,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
    );
    p.add(shape);
}

pub fn initial(s: &str) -> String {
    s.trim()
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Small rounded pill with a count (unread badge).
pub fn badge(
    p: &egui::Painter,
    right_center: egui::Pos2,
    count: usize,
    fill: Color32,
    text: Color32,
) {
    let label = if count > 999 {
        "999+".to_string()
    } else {
        count.to_string()
    };
    let galley = p.layout_no_wrap(label, bold_font(10.5), text);
    let w = galley.size().x + 10.0;
    let h = 16.0;
    let rect = egui::Rect::from_min_size(
        egui::pos2(right_center.x - w, right_center.y - h / 2.0),
        egui::vec2(w, h),
    );
    p.rect_filled(rect, egui::CornerRadius::same(8), fill);
    let pos = egui::pos2(rect.left() + 5.0, rect.center().y - galley.size().y / 2.0);
    p.galley(pos, galley, text);
}

/// One line of text clipped to `width` with an ellipsis.
pub fn clipped_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    width: f32,
    text: &str,
    font: egui::FontId,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap.max_width = width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter.galley(pos, galley, color);
}

pub fn hline(ui: &mut egui::Ui) {
    let t = tokens(ui);
    let r = ui.available_rect_before_wrap();
    let y = ui.cursor().top();
    ui.painter().line_segment(
        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
        egui::Stroke::new(1.0, t.border),
    );
    ui.add_space(1.0);
}
