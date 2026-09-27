//! Slim sponsor banner. Selection and measurement live in `crate::sponsor`.

use super::theme::{lerp_color, with_alpha, Theme};
use super::widgets::{self, bold, cr, font, Icon};
use crate::sponsor::{Ad, Choice};
use crate::i18n::tr;
use eframe::egui::{self, Align2, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2};

fn icon_for(ad: &Ad) -> Icon {
    match ad.icon.as_str() {
        "cloud" => Icon::Cloud,
        "server" => Icon::Server,
        "disk" => Icon::Disk,
        "shield" => Icon::Shield,
        "bolt" => Icon::Bolt,
        "folder" => Icon::Folder,
        "download" => Icon::Download,
        _ => Icon::Acorn,
    }
}

pub enum BannerAction {
    None,
    Open,
}

/// Draws the banner in `rect` (about 52pt tall).
pub fn banner(ui: &mut Ui, rect: Rect, id: Id, choice: &Choice, personalized: bool, theme: &Theme) -> BannerAction {
    let ad = &choice.ad;
    // "why" badge on the right, before the arrow
    let arrow = Rect::from_center_size(Pos2::new(rect.right() - 20.0, rect.center().y), Vec2::splat(14.0));
    let label_w = ui.painter().layout_no_wrap(tr("Sponsor").into(), font(10.5), theme.text_faint).size().x;
    let why_rect = Rect::from_min_max(Pos2::new(rect.right() - 38.0 - label_w - 22.0, rect.top()), Pos2::new(rect.right() - 32.0, rect.bottom()));
    let why = ui.interact(why_rect, id.with("why"), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.12);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    p.rect_filled(rect, cr(10.0), lerp_color(with_alpha(theme.surface, 0.72), theme.surface_hi, h * 0.7));
    p.rect_stroke(rect, cr(10.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.6)), egui::StrokeKind::Inside);
    let tile = Rect::from_center_size(Pos2::new(rect.left() + 26.0, rect.center().y), Vec2::splat(32.0));
    p.rect_filled(tile, cr(8.0), with_alpha(theme.accent, 0.18));
    widgets::draw_icon(p, icon_for(ad), tile.shrink(8.0), theme.accent);
    widgets::draw_icon(p, Icon::LinkOut, arrow, lerp_color(theme.text_dim, theme.text, h));
    let label_x = rect.right() - 38.0;
    let why_hot = why.hovered();
    p.text(Pos2::new(label_x, rect.center().y), Align2::RIGHT_CENTER, tr("Sponsor"), font(10.5), if why_hot { theme.text } else { theme.text_faint });
    // tiny "i" badge: hover to see why this sponsor was picked
    let info = Pos2::new(label_x - label_w - 10.0, rect.center().y);
    p.circle_stroke(info, 6.0, Stroke::new(1.0, if why_hot { theme.text } else { theme.text_faint }));
    p.text(info, Align2::CENTER_CENTER, "i", bold(9.0), if why_hot { theme.text } else { theme.text_faint });
    let tx = tile.right() + 12.0;
    let tw = (info.x - 14.0 - tx).max(40.0);
    let title = widgets::truncate(p, &ad.title, &bold(12.5), tw);
    if ad.text.trim().is_empty() {
        p.text(Pos2::new(tx, rect.center().y), Align2::LEFT_CENTER, title, bold(12.5), theme.text);
    } else {
        p.text(Pos2::new(tx, rect.center().y - 8.0), Align2::LEFT_CENTER, title, bold(12.5), theme.text);
        let text = widgets::truncate(p, &ad.text, &font(11.5), tw);
        p.text(Pos2::new(tx, rect.center().y + 9.0), Align2::LEFT_CENTER, text, font(11.5), theme.text_dim);
    }
    if why_hot {
        why_card(ui.ctx(), why_rect, choice, personalized, theme);
    }
    if resp.clicked() && !why_hot {
        BannerAction::Open
    } else {
        BannerAction::None
    }
}

/// "Why am I seeing this?" card, drawn above the banner.
fn why_card(ctx: &egui::Context, anchor: Rect, choice: &Choice, personalized: bool, theme: &Theme) {
    let mut lines: Vec<(String, bool)> = Vec::new();
    if choice.ad.is_house() {
        lines.push((tr("No sponsor right now: this is our own ad slot.").into(), false));
    } else if choice.reasons.is_empty() {
        lines.push((tr("Shown to everyone, not targeted.").into(), false));
    } else {
        lines.push((tr("Picked on this computer because:").into(), true));
        for r in &choice.reasons {
            lines.push((format!("\u{2022} {r}"), false));
        }
    }
    lines.push((String::new(), false));
    lines.push((tr("Nothing about you or your files is sent to us or to the sponsor.").into(), false));
    lines.push((tr("Every user downloads the same list of sponsors;").into(), false));
    lines.push((tr("the choice is made locally.").into(), false));
    if !personalized {
        lines.push((tr("Personalized sponsors are off (Settings).").into(), false));
    }
    let w = 340.0;
    let h = 24.0 + lines.len() as f32 * 17.0;
    let screen = ctx.content_rect();
    let x = (anchor.right() - w).clamp(screen.left() + 8.0, screen.right() - w - 8.0);
    let rect = Rect::from_min_size(Pos2::new(x, anchor.top() - h - 8.0), Vec2::new(w, h));
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("why-ad")));
    widgets::shadow(&p, rect, 10.0, 1.0, theme);
    p.rect_filled(rect, cr(10.0), theme.surface_hi);
    p.rect_stroke(rect, cr(10.0), Stroke::new(1.0, theme.stroke), egui::StrokeKind::Inside);
    let mut y = rect.top() + 14.0;
    for (text, strong) in lines {
        let f = if strong { bold(12.0) } else { font(12.0) };
        p.text(Pos2::new(rect.left() + 14.0, y), Align2::LEFT_CENTER, text, f, if strong { theme.text } else { theme.text_dim });
        y += 17.0;
    }
}
