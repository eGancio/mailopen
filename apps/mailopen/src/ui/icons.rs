//! Icons drawn in code (original, no asset files). Each is a few strokes in a unit square.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, vec2};

use crate::ui::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Compose,
    Search,
    Refresh,
    Inbox,
    Draft,
    Sent,
    Archive,
    Spam,
    Trash,
    Reply,
    ReplyAll,
    Forward,
    Check,
    Plus,
    Attachment,
    Folder,
    Mails,
    Close,
    Gear,
    Sun,
    Moon,
    Download,
    Print,
    OpenExternal,
    ZoomIn,
    ZoomOut,
    Fit,
    Flag,
    Envelope,
    Tag,
    Document,
    MoveTo,
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let origin = rect.center() - vec2(s / 2.0, s / 2.0);
    let pt = |x: f32, y: f32| -> Pos2 { origin + vec2(x * s, y * s) };
    let stroke = Stroke::new((s / 11.0).max(1.2), color);
    let line = |a: (f32, f32), b: (f32, f32)| p.line_segment([pt(a.0, a.1), pt(b.0, b.1)], stroke);
    let path = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            pts.iter().map(|(x, y)| pt(*x, *y)).collect(),
            stroke,
        ));
    };
    let closed = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::closed_line(
            pts.iter().map(|(x, y)| pt(*x, *y)).collect(),
            stroke,
        ));
    };
    let arc = |cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32| {
        let n = 14;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let t = from_deg + (to_deg - from_deg) * (i as f32 / n as f32);
                let a = t.to_radians();
                pt(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        p.add(egui::Shape::line(pts, stroke));
    };
    match icon {
        Icon::Compose => {
            path(&[
                (0.55, 0.15),
                (0.15, 0.15),
                (0.15, 0.85),
                (0.85, 0.85),
                (0.85, 0.45),
            ]);
            line((0.4, 0.6), (0.9, 0.1));
            line((0.4, 0.6), (0.38, 0.72));
        }
        Icon::Search => {
            p.circle_stroke(pt(0.42, 0.42), 0.28 * s, stroke);
            line((0.63, 0.63), (0.92, 0.92));
        }
        Icon::Refresh => {
            arc(0.5, 0.5, 0.36, -60.0, 230.0);
            line((0.68, 0.15), (0.72, 0.4));
            line((0.72, 0.4), (0.48, 0.42));
        }
        Icon::Inbox => {
            closed(&[(0.12, 0.15), (0.88, 0.15), (0.88, 0.85), (0.12, 0.85)]);
            path(&[
                (0.12, 0.58),
                (0.32, 0.58),
                (0.4, 0.7),
                (0.6, 0.7),
                (0.68, 0.58),
                (0.88, 0.58),
            ]);
        }
        Icon::Draft => {
            closed(&[(0.2, 0.1), (0.65, 0.1), (0.8, 0.25), (0.8, 0.9), (0.2, 0.9)]);
            line((0.33, 0.48), (0.67, 0.48));
            line((0.33, 0.63), (0.67, 0.63));
        }
        Icon::Sent => {
            closed(&[(0.1, 0.45), (0.9, 0.1), (0.62, 0.9), (0.46, 0.56)]);
            line((0.46, 0.56), (0.9, 0.1));
        }
        Icon::Archive => {
            closed(&[(0.1, 0.15), (0.9, 0.15), (0.9, 0.35), (0.1, 0.35)]);
            path(&[(0.17, 0.35), (0.17, 0.9), (0.83, 0.9), (0.83, 0.35)]);
            line((0.4, 0.55), (0.6, 0.55));
        }
        Icon::Spam => {
            p.circle_stroke(pt(0.5, 0.5), 0.38 * s, stroke);
            line((0.24, 0.24), (0.76, 0.76));
        }
        Icon::Trash => {
            line((0.15, 0.25), (0.85, 0.25));
            path(&[(0.38, 0.25), (0.38, 0.13), (0.62, 0.13), (0.62, 0.25)]);
            closed(&[(0.22, 0.25), (0.78, 0.25), (0.72, 0.9), (0.28, 0.9)]);
            line((0.42, 0.4), (0.44, 0.76));
            line((0.58, 0.4), (0.56, 0.76));
        }
        Icon::Reply => {
            path(&[(0.42, 0.18), (0.14, 0.42), (0.42, 0.66)]);
            path(&[(0.14, 0.42), (0.6, 0.42), (0.86, 0.58), (0.86, 0.88)]);
        }
        Icon::ReplyAll => {
            path(&[(0.5, 0.18), (0.24, 0.42), (0.5, 0.66)]);
            path(&[(0.3, 0.18), (0.06, 0.42), (0.3, 0.66)]);
            path(&[(0.24, 0.42), (0.62, 0.42), (0.88, 0.58), (0.88, 0.88)]);
        }
        Icon::Forward => {
            path(&[(0.58, 0.18), (0.86, 0.42), (0.58, 0.66)]);
            path(&[(0.86, 0.42), (0.4, 0.42), (0.14, 0.58), (0.14, 0.88)]);
        }
        Icon::Check => {
            path(&[(0.15, 0.52), (0.4, 0.78), (0.86, 0.26)]);
        }
        Icon::Plus => {
            line((0.5, 0.15), (0.5, 0.85));
            line((0.15, 0.5), (0.85, 0.5));
        }
        Icon::Gear => {
            p.circle_stroke(pt(0.5, 0.5), 0.17 * s, stroke);
            for i in 0..8 {
                let a = (45.0 * i as f32).to_radians();
                line(
                    (0.5 + 0.3 * a.cos(), 0.5 + 0.3 * a.sin()),
                    (0.5 + 0.42 * a.cos(), 0.5 + 0.42 * a.sin()),
                );
            }
            p.circle_stroke(pt(0.5, 0.5), 0.3 * s, stroke);
        }
        Icon::Sun => {
            p.circle_stroke(pt(0.5, 0.5), 0.2 * s, stroke);
            for i in 0..8 {
                let a = (45.0 * i as f32).to_radians();
                line(
                    (0.5 + 0.32 * a.cos(), 0.5 + 0.32 * a.sin()),
                    (0.5 + 0.45 * a.cos(), 0.5 + 0.45 * a.sin()),
                );
            }
        }
        Icon::Moon => {
            arc(0.5, 0.5, 0.38, 40.0, 320.0);
            arc(0.68, 0.38, 0.3, 110.0, 250.0);
        }
        Icon::Close => {
            line((0.2, 0.2), (0.8, 0.8));
            line((0.8, 0.2), (0.2, 0.8));
        }
        Icon::Attachment => {
            path(&[
                (0.36, 0.7),
                (0.36, 0.28),
                (0.5, 0.12),
                (0.64, 0.28),
                (0.64, 0.76),
                (0.5, 0.9),
                (0.36, 0.76),
                (0.36, 0.38),
                (0.5, 0.26),
                (0.5, 0.66),
            ]);
        }
        Icon::Folder => {
            closed(&[
                (0.1, 0.2),
                (0.4, 0.2),
                (0.5, 0.32),
                (0.9, 0.32),
                (0.9, 0.85),
                (0.1, 0.85),
            ]);
        }
        Icon::Mails => {
            closed(&[(0.25, 0.1), (0.95, 0.1), (0.95, 0.6), (0.25, 0.6)]);
            path(&[(0.25, 0.1), (0.6, 0.38), (0.95, 0.1)]);
            path(&[(0.1, 0.3), (0.1, 0.8), (0.8, 0.8)]);
        }
        Icon::Download => {
            line((0.5, 0.1), (0.5, 0.64));
            path(&[(0.26, 0.42), (0.5, 0.66), (0.74, 0.42)]);
            path(&[(0.12, 0.66), (0.12, 0.88), (0.88, 0.88), (0.88, 0.66)]);
        }
        Icon::Print => {
            path(&[(0.26, 0.38), (0.26, 0.1), (0.74, 0.1), (0.74, 0.38)]);
            closed(&[(0.1, 0.38), (0.9, 0.38), (0.9, 0.74), (0.1, 0.74)]);
            closed(&[(0.28, 0.6), (0.72, 0.6), (0.72, 0.92), (0.28, 0.92)]);
        }
        Icon::OpenExternal => {
            path(&[
                (0.44, 0.16),
                (0.12, 0.16),
                (0.12, 0.88),
                (0.84, 0.88),
                (0.84, 0.56),
            ]);
            line((0.46, 0.54), (0.88, 0.12));
            path(&[(0.6, 0.12), (0.88, 0.12), (0.88, 0.4)]);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            p.circle_stroke(pt(0.42, 0.42), 0.3 * s, stroke);
            line((0.64, 0.64), (0.92, 0.92));
            line((0.28, 0.42), (0.56, 0.42));
            if icon == Icon::ZoomIn {
                line((0.42, 0.28), (0.42, 0.56));
            }
        }
        Icon::Flag => {
            line((0.24, 0.1), (0.24, 0.92));
            closed(&[
                (0.24, 0.12),
                (0.8, 0.12),
                (0.66, 0.31),
                (0.8, 0.5),
                (0.24, 0.5),
            ]);
        }
        Icon::Envelope => {
            closed(&[(0.1, 0.22), (0.9, 0.22), (0.9, 0.8), (0.1, 0.8)]);
            path(&[(0.1, 0.24), (0.5, 0.54), (0.9, 0.24)]);
        }
        Icon::Tag => {
            closed(&[
                (0.12, 0.12),
                (0.5, 0.12),
                (0.9, 0.52),
                (0.52, 0.9),
                (0.12, 0.5),
            ]);
            p.circle_stroke(pt(0.31, 0.31), 0.06 * s, stroke);
        }
        Icon::Document => {
            closed(&[
                (0.2, 0.08),
                (0.6, 0.08),
                (0.82, 0.3),
                (0.82, 0.92),
                (0.2, 0.92),
            ]);
            path(&[(0.6, 0.08), (0.6, 0.3), (0.82, 0.3)]);
            line((0.34, 0.52), (0.68, 0.52));
            line((0.34, 0.68), (0.68, 0.68));
        }
        Icon::MoveTo => {
            closed(&[
                (0.08, 0.2),
                (0.38, 0.2),
                (0.48, 0.32),
                (0.92, 0.32),
                (0.92, 0.86),
                (0.08, 0.86),
            ]);
            line((0.3, 0.6), (0.68, 0.6));
            path(&[(0.56, 0.48), (0.7, 0.6), (0.56, 0.72)]);
        }
        Icon::Fit => {
            path(&[(0.1, 0.34), (0.1, 0.1), (0.34, 0.1)]);
            path(&[(0.66, 0.1), (0.9, 0.1), (0.9, 0.34)]);
            path(&[(0.9, 0.66), (0.9, 0.9), (0.66, 0.9)]);
            path(&[(0.34, 0.9), (0.1, 0.9), (0.1, 0.66)]);
        }
    }
}

/// A flat icon button: hover background, optional tooltip.
pub fn button(ui: &mut Ui, icon: Icon, size: f32, color: Color32, tooltip: &str) -> Response {
    let pad = 7.0;
    let (rect, resp) =
        ui.allocate_exact_size(vec2(size + pad * 2.0, size + pad * 2.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(6), theme::tokens(ui).hover);
        }
        let icon_rect = Rect::from_center_size(rect.center(), vec2(size, size));
        paint(ui.painter(), icon_rect, icon, color);
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

pub fn small(ui: &mut Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui.painter(), rect, icon, color);
    }
}

/// Convenience for painting at a position.
pub fn at(p: &Painter, center: Pos2, size: f32, icon: Icon, color: Color32) {
    paint(
        p,
        Rect::from_center_size(center, vec2(size, size)),
        icon,
        color,
    );
}
