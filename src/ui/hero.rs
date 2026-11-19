//! The hero that opens Home — Mochy's Mochyy home style, ported: full-bleed cover
//! art sixty percent of the screen tall, auto-advancing every five and a half
//! seconds (a gacha page holds double — there's a roll to watch), with what's
//! playing or last played (Continue), the gacha card that rolls a random play,
//! discoveries grown from what you like, and your playlists.

use egui::{Color32, Rect, Sense, Ui, pos2, vec2};

use super::widgets::{self, icon_at};
use super::{Action, App};
use crate::model::{Card, Target, Track};
use crate::theme::{self, icon};

/// How long a card holds before the next wipes in (Mochy's `HeroAdvanceMillis`).
const ADVANCE: f64 = 5.5;
/// A gacha page holds double: the roll shouldn't be walked away from mid-spin.
const GACHA_ADVANCE: f64 = 11.0;
/// A roll spins this long: three and a half to nine seconds.
const SPIN_MIN: f64 = 3.5;
const SPIN_MAX: f64 = 9.0;
/// The reel's covers cycle this fast while it spins.
const REEL: f64 = 8.0;
/// A gacha needs this many songs to roll from.
const GACHA_MIN_POOL: usize = 4;

/// A gacha roll under way: what it's spinning through, what it landed on, and when.
pub(super) struct Gacha {
    pub until: f64,
    pub won: Track,
    pool: Vec<Track>,
}

/// One card of the pager.
enum CardKind {
    /// What's playing or was last played.
    Continue(Track),
    /// A discovery; the label line says what it grew from.
    Discover { seed: Track, found: Track },
    /// One of your playlists.
    Playlist(Card),
    /// A keep-listening song, filling the pager out.
    Keep(Track),
}

/// The hero's cards, as the page opens with them.
fn cards(app: &App) -> Vec<CardKind> {
    let mut cards = Vec::new();
    // what's on now, else the last thing played
    let last = app.current().cloned().or_else(|| app.listening.recently_played(1).into_iter().next());
    if let Some(t) = last {
        cards.push(CardKind::Continue(t));
    }
    for (seed, found) in app.discover.iter().take(4) {
        cards.push(CardKind::Discover { seed: seed.clone(), found: found.clone() });
    }
    // your playlists, the library's (the sidebar's stand in until it arrives)
    let playlists: Vec<Card> = app
        .library
        .as_ref()
        .map(|l| l.playlists.clone())
        .or_else(|| app.sidebar_cache.clone())
        .unwrap_or_default();
    for c in playlists.iter().take(2) {
        cards.push(CardKind::Playlist(c.clone()));
    }
    // filling out the pager with what to pick back up
    let keep: Vec<Track> = app
        .home_rows
        .keep
        .iter()
        .filter_map(|c| match &c.target {
            Target::Song(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    for t in keep {
        if cards.len() >= 6 {
            break;
        }
        if !matches!(cards.first(), Some(CardKind::Continue(t2)) if t2.id == t.id) {
            cards.push(CardKind::Keep(t));
        }
    }
    cards
}

pub(super) fn draw(app: &mut App, ui: &mut Ui, now: f64) {
    let cards = cards(app);
    let gacha_slot = app.gacha_pool.len() >= GACHA_MIN_POOL;
    let pages = cards.len() + gacha_slot as usize;
    if pages == 0 {
        return;
    }

    // A settled spin fires and clears, before anything else draws.
    if let Some(g) = app.gacha.take() {
        if now >= g.until {
            let won = g.won;
            app.toast(format!("Rolled \"{}\"", won.title));
            app.actions.push(Action::Radio(won));
        } else {
            app.gacha = Some(g);
        }
    }
    let spinning = app.gacha.is_some();
    if spinning {
        // the reel needs its frames
        ui.ctx().request_repaint();
    }

    // Sixty percent of the screen, the way Mochy's hero sizes itself.
    let screen_h = ui.ctx().input(|i| i.raw.screen_rect.map_or(800.0, |r| r.height()));
    let height = (screen_h * 0.6).clamp(260.0, 520.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return;
    }
    let page = app.hero_page % pages;
    let on_gacha = gacha_slot && page == 1;
    // page 0 is the first card; with a gacha, page 1 is its card, the rest follow
    let kind = if on_gacha {
        None
    } else {
        let idx = page - if gacha_slot && page > 1 { 1 } else { 0 };
        cards.get(idx)
    };

    let advance = if on_gacha { GACHA_ADVANCE } else { ADVANCE };
    // Not while the pointer is on it (a card being read) or a roll is spinning.
    if pages > 1 && !spinning && !resp.contains_pointer() && now - app.hero_at > advance {
        app.hero_page = (page + 1) % pages;
        app.hero_at = now;
        // the incoming card wipes in from the right
        ui.ctx().animate_value_with_time(slide_id(ui), rect.width(), 0.0);
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32((0.5f64).max(advance - (now - app.hero_at)) as f32));

    ui.painter().rect_filled(rect, theme::card_radius(), theme::shade(0.35));
    match kind {
        Some(CardKind::Continue(t)) => cover_slide(ui, rect, &t.thumbs),
        Some(CardKind::Discover { found, .. }) => cover_slide(ui, rect, &found.thumbs),
        Some(CardKind::Playlist(c)) => cover_slide(ui, rect, &c.thumbs),
        Some(CardKind::Keep(t)) => cover_slide(ui, rect, &t.thumbs),
        // the gacha card: the reel while a roll spins, a pool cover otherwise
        None => {
            let reel = app
                .gacha
                .as_ref()
                .and_then(|g| g.pool.get(((now * REEL) as usize) % g.pool.len()))
                .or_else(|| app.gacha_pool.first());
            if let Some(t) = reel {
                cover_slide(ui, rect, &t.thumbs);
            }
            // its cover reads as one card, dimmed under the roll
            ui.painter().rect_filled(rect, theme::card_radius(), theme::shade(0.45));
        }
    }

    // the scrim the label and title sit on
    vgradient(ui.painter(), Rect::from_min_size(rect.left_bottom(), vec2(rect.width(), rect.height() * 0.5)));

    // label line, title, and the play pill, bottom-left
    let left = rect.left() + 28.0;
    let base = rect.bottom() - 44.0;
    let playing = app.current().is_some() && matches!(kind, Some(CardKind::Continue(_)));
    let (label, title, action) = match kind {
        Some(CardKind::Continue(t)) => ((if app.current().is_some() { "Now playing" } else { "Continue" }).to_owned(), t.title.clone(), Some(Action::Radio(t.clone()))),
        Some(CardKind::Discover { seed, found }) => (format!("Because you like {}", seed.title), found.title.clone(), Some(Action::Radio(found.clone()))),
        Some(CardKind::Playlist(c)) => ("From your playlists".to_owned(), c.title.clone(), None),
        Some(CardKind::Keep(t)) => ("Pick back up".to_owned(), t.title.clone(), Some(Action::Radio(t.clone()))),
        None => ("Random plays".to_owned(), String::new(), None),
    };
    // the label's mini cover, round, beside its text (a discovery names its seed)
    let mut label_x = left;
    if let Some(CardKind::Discover { seed, .. }) = kind {
        if let Some(url) = crate::model::pick(&seed.thumbs, 120) {
            let r = Rect::from_center_size(pos2(left + 10.0, base - 26.0), vec2(20.0, 20.0));
            widgets::paint_remote(ui, url, r, 10.0, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        label_x = left + 26.0;
    }
    if !label.is_empty() {
        ui.painter().text(pos2(label_x, base - 16.0), egui::Align2::LEFT_BOTTOM, label, theme::semibold(14.0), theme::with_alpha(theme::text(), 0.7));
    }
    if !title.is_empty() {
        ui.painter().text(pos2(left, base + 2.0), egui::Align2::LEFT_BOTTOM, &title, theme::bold(26.0), theme::text());
    }

    // the pill: Play for a song (Pause on the one playing), Roll on the gacha card
    let acts = &mut app.actions;
    let pill_y = base + 12.0;
    let mut rolled = false;
    match kind {
        None if spinning => {
            ui.painter().text(pos2(left, pill_y + 17.0), egui::Align2::LEFT_CENTER, "Rolling…", theme::semibold(15.0), theme::text());
        }
        None => {
            rolled = pill_at(ui, Rect::from_min_size(pos2(left, pill_y), vec2(96.0, 34.0)), "Roll", Some(icon::SPARKLES), false).clicked();
        }
        Some(CardKind::Playlist(c)) => {
            if pill_at(ui, Rect::from_min_size(pos2(left, pill_y), vec2(96.0, 34.0)), "Play", Some(icon::PLAY), true).clicked() {
                acts.push(Action::PlayCollection(c.target.clone(), false));
            }
        }
        _ => {
            let (name, glyph) = if playing { ("Pause", icon::PAUSE) } else { ("Play", icon::PLAY) };
            if pill_at(ui, Rect::from_min_size(pos2(left, pill_y), vec2(96.0, 34.0)), name, Some(glyph), true).clicked() {
                if playing {
                    acts.push(Action::TogglePlay);
                } else if let Some(a) = action {
                    acts.push(a);
                }
            }
        }
    }
    if rolled && app.gacha_pool.len() >= GACHA_MIN_POOL {
        // pick from the pool with chance: the reel spins what it spins through, and
        // the roll takes 3.5–9 s before it lands
        let mut rng = crate::recommend::Rng::new();
        let won = app.gacha_pool[(rng.unit() * app.gacha_pool.len() as f64) as usize].clone();
        let spin = SPIN_MIN + rng.unit() * (SPIN_MAX - SPIN_MIN);
        app.gacha = Some(Gacha { until: now + spin, won, pool: app.gacha_pool.clone() });
        app.hero_at = now;
    }

    // clicking the card plays (a playlist plays all of it); the pill takes its own clicks
    if resp.clicked() && !spinning {
        match kind {
            Some(CardKind::Playlist(c)) => acts.push(Action::PlayCollection(c.target.clone(), false)),
            _ if playing => acts.push(Action::TogglePlay),
            _ => {
                let track = match kind {
                    Some(CardKind::Continue(t)) | Some(CardKind::Keep(t)) => Some(t.clone()),
                    Some(CardKind::Discover { found, .. }) => Some(found.clone()),
                    _ => None,
                };
                if let Some(t) = track {
                    acts.push(Action::Radio(t));
                }
            }
        }
    }
    // the pager's arrows and dots, only with something to page through
    if pages > 1 {
        let (l, r) = pager_arrows(ui, rect);
        if l {
            app.hero_page = (page + pages - 1) % pages;
            app.hero_at = now;
            ui.ctx().animate_value_with_time(slide_id(ui), -rect.width(), 0.0);
        }
        if r {
            app.hero_page = (page + 1) % pages;
            app.hero_at = now;
            ui.ctx().animate_value_with_time(slide_id(ui), rect.width(), 0.0);
        }
        let dot_y = rect.bottom() - 10.0;
        for p in 0..pages {
            let on = p == page;
            ui.painter().circle_filled(pos2(rect.left() + 28.0 + p as f32 * 16.0, dot_y), if on { 4.0 } else { 2.5 }, theme::with_alpha(theme::text(), if on { 0.9 } else { 0.45 }));
        }
    }
}

/// The wipe: the incoming cover slides in over the last one's dark underlay.
fn cover_slide(ui: &Ui, rect: Rect, thumbs: &[crate::model::Thumb]) {
    let slide = ui.ctx().animate_value_with_time(slide_id(ui), 0.0, 1.15 * theme::speed());
    // under it, the cover holds — the wipe never shows a bare background
    widgets::paint_cover_fill(ui, rect, thumbs, Color32::WHITE);
    if slide.abs() > 0.5 {
        let shifted = Rect::from_min_size(pos2(rect.left() + slide, rect.top()), rect.size());
        widgets::paint_cover_fill(ui, shifted, thumbs, Color32::WHITE);
        ui.ctx().request_repaint();
    }
}

fn slide_id(ui: &Ui) -> egui::Id {
    egui::Id::unique(("hero-slide", ui.scope_id()))
}

/// Vertical gradient over the hero's lower half, for the label and title to sit
/// on — the corner-colour mesh `shell::gradient` builds, stood on end.
fn vgradient(painter: &egui::Painter, rect: Rect) {
    let (top, bottom) = (Color32::from_black_alpha(0), Color32::from_black_alpha(210));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

/// A pill at a rect of the hero's own — `widgets::pill` lays itself out in a
/// panel's flow; the hero pins its own. Same look: white when primary.
fn pill_at(ui: &mut Ui, rect: Rect, label: &str, glyph: Option<char>, primary: bool) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::unique(("hero-pill", label)), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let h = theme::anim_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), 0.15);
    let (fill, fg) = if primary {
        if theme::light() { (theme::lerp_color(theme::text(), Color32::BLACK, h), Color32::WHITE) } else { (theme::lerp_color(Color32::from_gray(240), Color32::WHITE, h), Color32::from_gray(12)) }
    } else {
        (theme::glass(0.10 + 0.08 * h), theme::text())
    };
    let r = rect.shrink(if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 });
    ui.painter().rect_filled(r, r.height() / 2.0, fill);
    let text = ui.painter().layout_no_wrap(label.to_owned(), theme::semibold(14.0), Color32::PLACEHOLDER);
    let icon_w = if glyph.is_some() { 24.0 } else { 0.0 };
    let mut x = r.left() + (r.width() - text.size().x - icon_w) / 2.0;
    if let Some(g) = glyph {
        icon_at(ui, pos2(x + 8.0, r.center().y), g, 15.0, fg);
        x += icon_w;
    }
    ui.painter().galley(pos2(x, r.center().y - text.size().y / 2.0), text, fg);
    resp
}

/// The pager's ‹ › on the hero's right edge, halfway up.
fn pager_arrows(ui: &mut Ui, rect: Rect) -> (bool, bool) {
    let mid = rect.center().y;
    let mut clicked = (false, false);
    for (i, (glyph, name)) in [(icon::CHEVRON_RIGHT, "Next card"), (icon::CHEVRON_LEFT, "Previous card")].into_iter().enumerate() {
        let center = pos2(rect.right() - 22.0 - i as f32 * 40.0, mid);
        let hit = Rect::from_center_size(center, vec2(34.0, 34.0));
        let resp = ui.interact(hit, egui::Id::unique(("hero-arrow", i)), Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(name);
        let h = theme::anim_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), 0.15);
        if h > 0.0 {
            ui.painter().circle_filled(center, 17.0, theme::glass(0.10 * h));
        }
        icon_at(ui, center, glyph, 18.0, theme::lerp_color(theme::text_dim(), theme::text(), h));
        if resp.clicked() {
            clicked.1 = i == 0;
            clicked.0 = i == 1;
        }
    }
    clicked
}
