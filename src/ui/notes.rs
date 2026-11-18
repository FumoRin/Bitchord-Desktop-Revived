//! Release notes: a small renderer for GitHub release bodies (headings, paragraphs,
//! bullets, **bold**, `code`, [links](…) and bare URLs), the dialog shown when an
//! update is ready (or to read a version's notes), and the card shown after updating.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Margin, Stroke, Ui, vec2};

use super::{App, widgets};
use crate::theme::{self, icon};

pub enum Block {
    Heading(String),
    Para(String),
    Bullet(String),
}

pub fn parse(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para = String::new();
    let flush = |para: &mut String, out: &mut Vec<Block>| {
        if !para.trim().is_empty() {
            out.push(Block::Para(para.trim().to_owned()));
        }
        para.clear();
    };
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut para, &mut out);
        } else if t.starts_with('#') {
            flush(&mut para, &mut out);
            out.push(Block::Heading(t.trim_start_matches('#').trim().to_owned()));
        } else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush(&mut para, &mut out);
            out.push(Block::Bullet(b.trim().to_owned()));
        } else if let (true, Some(Block::Bullet(b))) = (para.is_empty(), out.last_mut()) {
            // A wrapped bullet continues on the next line.
            b.push(' ');
            b.push_str(t);
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t);
        }
    }
    flush(&mut para, &mut out);
    out
}

/// Inline spans: (text, bold, and the URL when it's a link). Links keep their text;
/// code loses its backticks. A bare URL (`https://…`) is a link too, with the
/// punctuation that usually trails one left out of it.
fn spans(s: &str) -> Vec<(String, bool, Option<String>)> {
    let mut out: Vec<(String, bool, Option<String>)> = Vec::new();
    let mut push = |t: &str, bold: bool, url: Option<String>| match (out.last_mut(), &url) {
        // Only plain runs of one boldness merge; every link is its own span.
        (Some((prev, b, None)), None) if *b == bold => prev.push_str(t),
        _ => out.push((t.to_owned(), bold, url)),
    };
    let mut rest = s;
    let mut bold = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("**") {
            bold = !bold;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('`') {
            let end = r.find('`').unwrap_or(r.len());
            push(&r[..end], bold, None);
            rest = r.get(end + 1..).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix('[')
            && let Some(close) = r.find("](")
            && let Some(end) = r[close..].find(')')
        {
            push(&r[..close], bold, Some(r[close + 2..close + end].to_owned()));
            rest = &r[close + end + 1..];
        } else {
            let next = rest.char_indices().skip(1).find(|(_, c)| matches!(c, '*' | '`' | '[')).map_or(rest.len(), |(i, _)| i);
            let run = &rest[..next];
            match bare_link(run) {
                // Text up to the URL, the URL as a link, then the rest goes round again
                // (its punctuation first, glued back on in `words`).
                Some((off, url, after)) => {
                    if off > 0 {
                        push(&run[..off], bold, None);
                    }
                    push(url, bold, Some(url.to_owned()));
                    rest = after;
                }
                None => {
                    push(run, bold, None);
                    rest = &rest[next..];
                }
            }
        }
    }
    out
}

/// The first bare URL in `s`, starting a word (after a space, a bracket or the like):
/// where it starts, the URL without the punctuation that trails it, and the rest from
/// there. `None` when there's no URL before the line's next markdown.
fn bare_link(s: &str) -> Option<(usize, &str, &str)> {
    let mut best: Option<usize> = None;
    for prefix in ["https://", "http://"] {
        let mut from = 0;
        while let Some(found) = s[from..].find(prefix) {
            let at = from + found;
            let boundary = s[..at].chars().next_back().is_none_or(|c| c.is_whitespace() || "(*_~".contains(c));
            if boundary {
                best = Some(best.map_or(at, |b: usize| b.min(at)));
                break;
            }
            from = at + 1;
        }
    }
    let at = best?;
    let end = s[at..].find(char::is_whitespace).map_or(s.len(), |e| at + e);
    let url = s[at..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '\'', '"']);
    Some((at, url, &s[at + url.len()..]))
}

/// The line's words, each the pieces it's made of: bold beside plain, a link's words,
/// a bare URL's trailing period glued back on. Between words there was a space;
/// between the pieces of one there wasn't.
fn words(s: &str) -> Vec<Vec<(String, bool, Option<String>)>> {
    let mut out: Vec<Vec<(String, bool, Option<String>)>> = Vec::new();
    let mut word: Vec<(String, bool, Option<String>)> = Vec::new();
    // Whether what comes next starts a word of its own: a space just went by, or
    // nothing has been written yet.
    let mut gap = true;
    let mut buf = String::new();
    let flush = |buf: &mut String, bold: bool, gap: &mut bool, word: &mut Vec<(String, bool, Option<String>)>, out: &mut Vec<Vec<(String, bool, Option<String>)>>| {
        if buf.is_empty() {
            return;
        }
        if !*gap && word.is_empty()
            && let Some(last) = out.last_mut()
        {
            // No space since the last word: this piece is part of it — a bare URL's
            // ".", text a link ran into.
            last.push((std::mem::take(buf), bold, None));
        } else {
            if *gap && !word.is_empty() {
                out.push(std::mem::take(word));
            }
            word.push((std::mem::take(buf), bold, None));
        }
        *gap = false;
    };
    for (t, bold, url) in spans(s) {
        if let Some(u) = url {
            flush(&mut buf, bold, &mut gap, &mut word, &mut out);
            // A link's own text is words of its own, each opening the same place.
            for w in t.split_whitespace() {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                out.push(vec![(w.to_owned(), bold, Some(u.clone()))]);
            }
            gap = false;
            continue;
        }
        for c in t.chars() {
            if c.is_whitespace() {
                flush(&mut buf, bold, &mut gap, &mut word, &mut out);
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                gap = true;
            } else {
                buf.push(c);
            }
        }
        flush(&mut buf, bold, &mut gap, &mut word, &mut out);
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn plain(s: &str) -> String {
    spans(s).into_iter().map(|(t, _, _)| t).collect()
}

fn job(text: &str, size: f32, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for (t, bold, _) in spans(text) {
        let font = if bold { theme::semibold(size) } else { theme::regular(size) };
        let color = if bold { theme::text() } else { color };
        job.append(&t, 0.0, TextFormat { font_id: font, color, line_height: Some(size * 1.5), ..Default::default() });
    }
    job
}

/// Whether a line carries a link (markdown or bare), and so needs the word-by-word
/// layout that makes each link its own widget.
fn linked(s: &str) -> bool {
    spans(s).iter().any(|(_, _, url)| url.is_some())
}

/// The notes laid out like the release page: section headings, paragraphs, bullets.
pub fn show(ui: &mut Ui, blocks: &[Block], size: f32) {
    let width = ui.available_width();
    let body = body_color();
    for (i, b) in blocks.iter().enumerate() {
        match b {
            Block::Heading(t) => {
                if i > 0 {
                    ui.add_space(12.0);
                }
                let g = widgets::galley(ui, &plain(t), theme::semibold(size + 1.0), theme::text(), width);
                ui.label(g);
                ui.add_space(4.0);
            }
            Block::Para(t) => {
                if linked(t) {
                    rich(ui, t, size, body, width, false);
                } else {
                    let g = ui.fonts_mut(|f| f.layout_job(job(t, size, body, width)));
                    ui.label(g);
                }
                ui.add_space(6.0);
            }
            Block::Bullet(t) => {
                if linked(t) {
                    rich(ui, t, size, body, width, true);
                } else {
                    let g = ui.fonts_mut(|f| f.layout_job(job(t, size, body, width - 18.0)));
                    let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), egui::Sense::hover());
                    let dot = egui::pos2(rect.left() + 6.0, rect.top() + size * 0.75);
                    ui.painter().circle_filled(dot, 2.2, body);
                    ui.painter().galley(rect.min + vec2(18.0, 0.0), g, body);
                }
                ui.add_space(4.0);
            }
        }
    }
}

/// A paragraph or bullet that carries links, laid out word by word so each link is
/// its own widget: the same fonts, colors and line height as the galley-built lines,
/// with a real space's width between the words.
fn rich(ui: &mut Ui, text: &str, size: f32, color: Color32, width: f32, bullet: bool) {
    let top = ui.cursor().min;
    let space = ui.painter().layout_no_wrap(" ".to_owned(), theme::regular(size), color).size().x;
    ui.horizontal(|ui| {
        if bullet {
            ui.add_space(18.0);
        }
        ui.vertical(|ui| {
            ui.set_width(width - if bullet { 18.0 } else { 0.0 });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(space, 0.0);
                for word in words(text) {
                    // One word: its pieces sit tight against each other, whatever mixes
                    // inside it (a bare URL and its trailing period).
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (t, bold, url) in word {
                            let font = if bold { theme::semibold(size) } else { theme::regular(size) };
                            let mut j = LayoutJob::default();
                            j.append(&t, 0.0, TextFormat { font_id: font, color: if bold { theme::text() } else { color }, line_height: Some(size * 1.5), ..Default::default() });
                            match url {
                                Some(url) => {
                                    ui.hyperlink_to(j, url);
                                }
                                None => {
                                    ui.label(j);
                                }
                            }
                        }
                    });
                }
            });
        });
    });
    if bullet {
        ui.painter().circle_filled(egui::pos2(top.x + 6.0, top.y + size * 0.75), 2.2, color);
    }
}

/// The card's short list: feature headings and bullets (first sentence), up to
/// three, plus how many more there are. A "Fixes" heading isn't an item itself.
pub fn summary(blocks: &[Block]) -> (Vec<String>, usize) {
    let mut items = Vec::new();
    for b in blocks {
        match b {
            Block::Heading(t) if !t.to_lowercase().starts_with("fix") => items.push(plain(t)),
            Block::Bullet(t) => {
                let t = plain(t);
                let first = t.find(". ").map_or(t.as_str(), |i| &t[..i]);
                items.push(first.trim_end_matches('.').to_owned());
            }
            _ => {}
        }
    }
    let more = items.len().saturating_sub(3);
    items.truncate(3);
    (items, more)
}

/// The notes' paragraphs and bullets.
fn body_color() -> Color32 {
    if theme::light() { theme::text() } else { Color32::from_gray(225) }
}

/// Lines between parts of the card.
fn rule_color() -> Color32 {
    if theme::light() { Color32::from_gray(222) } else { Color32::from_gray(44) }
}

pub(super) fn surface() -> egui::Frame {
    egui::Frame::new()
        .fill(if theme::light() { theme::surface_fill() } else { Color32::from_gray(28) })
        .stroke(Stroke::new(1.0, rule_color()))
        .corner_radius(16)
        .shadow(egui::epaint::Shadow { offset: [0, 24], blur: 70, spread: 0, color: theme::shade(0.55) })
}

fn hline(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, rule_color());
}

pub fn github_link(ui: &mut Ui) {
    ui.hyperlink_to(egui::RichText::new("Full release notes on GitHub").font(theme::regular(13.0)).color(if theme::light() { Color32::from_rgb(0, 102, 204) } else { Color32::from_rgb(62, 166, 255) }), crate::update::RELEASES);
}

/// What the notes dialog shows: a downloaded update (with Restart), or a version's notes.
#[derive(Clone)]
pub struct NotesDialog {
    pub version: String,
    pub ready: bool,
}

pub fn dialog(app: &mut App, ctx: &egui::Context) {
    let Some(d) = app.notes_dialog.clone() else { return };
    let body = app.updater.as_ref().and_then(|u| u.notes(&d.version));
    // For a copy a package manager owns (the .deb or the Arch package), which can't
    // install it itself: the new package to download.
    let deb_url = app.updater.as_ref().and_then(|u| match u.state() {
        crate::update::State::Available { version, url } if d.ready && version == d.version => Some(url),
        _ => None,
    });
    let (mut close, mut restart) = (false, false);
    let accent = app.accent_color();
    let modal = egui::Modal::new(egui::Id::unique("release-notes")).frame(surface()).backdrop_color(theme::shade(0.55)).show(ctx, |ui| {
        let w = 540.0_f32.min(ctx.content_rect().width() - 48.0);
        ui.set_width(w);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        egui::Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 18 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::icon_at(ui, ui.cursor().min + vec2(8.0, 8.0), icon::SPARKLES, 15.0, accent);
                ui.add_space(22.0);
                let label = if deb_url.is_some() { "UPDATE AVAILABLE" } else if d.ready { "UPDATE READY" } else { "WHAT'S NEW" };
                let mut j = LayoutJob::default();
                j.append(label, 0.0, TextFormat { font_id: theme::semibold(12.0), color: theme::text_dim(), extra_letter_spacing: 1.0, ..Default::default() });
                ui.label(j);
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(format!("BitChord {}", d.version)).font(theme::bold(26.0)).color(theme::text()));
            ui.add_space(3.0);
            ui.label(egui::RichText::new("by Loserdenji").font(theme::semibold(13.0)).color(accent));
            ui.add_space(3.0);
            let sub = if deb_url.is_some() {
                format!("You're on {}. {}", crate::update::current_version(), crate::update::package_step())
            } else if d.ready {
                format!("You're on {}. It's downloaded and installs when you close the app.", crate::update::current_version())
            } else {
                "The version you're running.".to_owned()
            };
            ui.label(egui::RichText::new(sub).font(theme::regular(13.5)).color(theme::text_dim()));
        });
        hline(ui);
        let max_h = (ctx.content_rect().height() - 330.0).clamp(160.0, 460.0);
        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 28, right: 28, top: 18, bottom: 18 }).show(ui, |ui| {
                match &body {
                    Some(md) => show(ui, &parse(md), 14.0),
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Loading release notes…").color(theme::text_dim()));
                        });
                    }
                }
                ui.add_space(8.0);
                github_link(ui);
            });
        });
        hline(ui);
        egui::Frame::new().inner_margin(Margin { left: 28, right: 28, top: 16, bottom: 16 }).show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(url) = &deb_url {
                    if widgets::pill(ui, "Download", None, true).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        close = true;
                    }
                    close |= widgets::pill(ui, "Later", None, false).clicked();
                } else if d.ready {
                    restart = widgets::pill(ui, "Restart to update", None, true).clicked();
                    close = widgets::pill(ui, "Later", None, false).clicked();
                } else {
                    close = widgets::pill(ui, "Close", None, true).clicked();
                }
                // Where the news lands first, beside whatever else the bar offers.
                if widgets::pill(ui, "Join the Discord server", Some(icon::USERS), false).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(crate::discord::DISCORD_INVITE));
                }
            });
        });
    });
    if close || modal.should_close() {
        app.notes_dialog = None;
    }
    if restart {
        app.notes_dialog = None;
        app.restart_to_update(ctx);
    }
}

/// "Updated to X" in the corner on the first launch of a new version.
pub fn card(app: &mut App, ctx: &egui::Context) {
    let Some(version) = app.whats_new.clone() else { return };
    let Some(md) = app.updater.as_ref().and_then(|u| u.notes(&version)) else { return };
    let (items, more) = summary(&parse(&md));
    if items.is_empty() {
        app.whats_new = None;
        return;
    }
    let screen = ctx.content_rect();
    let at = egui::pos2(screen.right() - 24.0, screen.bottom() - app.bar_height() - 20.0);
    let t = theme::anim_bool(ctx, "whats-new-in", true, 0.35);
    let (mut dismiss, mut all) = (false, false);
    egui::Area::new(egui::Id::unique("whats-new"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(at + vec2(0.0, 16.0 * (1.0 - t)))
        .show(ctx, |ui| {
            ui.multiply_opacity(t);
            surface().corner_radius(14).inner_margin(Margin { left: 18, right: 12, top: 14, bottom: 16 }).show(ui, |ui| {
                ui.set_width(330.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(format!("Updated to {version}")).font(theme::semibold(15.0)).color(theme::text()));
                        ui.add_space(3.0);
                        ui.label(egui::RichText::new("Here's what changed.").font(theme::regular(13.0)).color(theme::text_dim()));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        dismiss = widgets::icon_button(ui, icon::X, 16.0, "Dismiss", false).clicked();
                    });
                });
                ui.add_space(10.0);
                let mut lines: Vec<String> = items.clone();
                if more > 0 {
                    lines.push(if more == 1 { "+ 1 more change".to_owned() } else { format!("+ {more} more changes") });
                }
                for (i, l) in lines.iter().enumerate() {
                    let faint = more > 0 && i == lines.len() - 1;
                    let color = if faint { theme::text_dim() } else { body_color() };
                    let mut j = LayoutJob::single_section(l.clone(), TextFormat { font_id: theme::regular(13.5), color, ..Default::default() });
                    j.wrap.max_width = 300.0;
                    j.wrap.max_rows = 1;
                    let g = ui.fonts_mut(|f| f.layout_job(j));
                    let (rect, _) = ui.allocate_exact_size(vec2(318.0, 22.0), egui::Sense::hover());
                    if !faint {
                        ui.painter().circle_filled(egui::pos2(rect.left() + 5.0, rect.center().y), 2.2, color);
                    }
                    ui.painter().galley(egui::pos2(rect.left() + 16.0, rect.center().y - g.size().y / 2.0), g, color);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    all = widgets::pill(ui, "See all notes", None, true).clicked();
                    dismiss |= widgets::pill(ui, "Got it", None, false).clicked();
                });
            });
        });
    if all {
        app.notes_dialog = Some(NotesDialog { version: version.clone(), ready: false });
    }
    if all || dismiss {
        app.whats_new = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "## Uploads\n\nPick **files** or drop them.\n\n## Fixes\n\n- Mixes play again. They broke in 0.2.9.\n- Icons are `centred`\n  in their buttons.\n- See [the notes](https://example.com).\n";

    #[test]
    fn parses_release_markdown() {
        let b = parse(MD);
        assert!(matches!(&b[0], Block::Heading(t) if t == "Uploads"));
        assert!(matches!(&b[1], Block::Para(t) if t == "Pick **files** or drop them."));
        assert!(matches!(&b[4], Block::Bullet(t) if t == "Icons are `centred` in their buttons."));
        assert_eq!(spans("Pick **files** now"), vec![("Pick ".into(), false, None), ("files".into(), true, None), (" now".into(), false, None)]);
        assert_eq!(plain("See [the notes](https://example.com)."), "See the notes.");
    }

    #[test]
    fn summary_takes_features_then_first_sentences() {
        let (items, more) = summary(&parse(MD));
        assert_eq!(items, vec!["Uploads", "Mixes play again", "Icons are centred in their buttons"]);
        assert_eq!(more, 1);
    }

    #[test]
    fn bare_urls_are_links_without_their_punctuation() {
        assert_eq!(spans("Read https://example.com/a?b=1 now"), vec![("Read ".into(), false, None), ("https://example.com/a?b=1".into(), false, Some("https://example.com/a?b=1".into())), (" now".into(), false, None)]);
        // The period after a URL stays plain text, glued onto it as one word.
        assert_eq!(spans("See https://example.com."), vec![("See ".into(), false, None), ("https://example.com".into(), false, Some("https://example.com".into())), (".".into(), false, None)]);
        assert_eq!(words("See https://example.com."), vec![
            vec![("See".into(), false, None)],
            vec![("https://example.com".into(), false, Some("https://example.com".into())), (".".into(), false, None)],
        ]);
        // An https inside a word isn't a link.
        assert!(spans("nothttps://example.com").iter().all(|(_, _, url)| url.is_none()));
    }

    #[test]
    fn markdown_links_span_words() {
        assert_eq!(words("See [the notes](https://example.com) here"), vec![
            vec![("See".into(), false, None)],
            vec![("the".into(), false, Some("https://example.com".into()))],
            vec![("notes".into(), false, Some("https://example.com".into()))],
            vec![("here".into(), false, None)],
        ]);
        assert!(linked("See [the notes](https://example.com) here"));
        assert!(!linked("Plain **bold** and `code` only"));
    }
}
