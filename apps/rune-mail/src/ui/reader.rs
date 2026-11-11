//! The message pane: a conversation's messages stacked, each an avatar, a header block and
//! the body as HTML or plain text. Messages that aren't open show as list rows (sender,
//! time, subject, two preview lines). Conversations get a map down the right edge, like an
//! editor's minimap: one block per message, as tall as its text is long, in the sender's
//! colour; the highlighted message is ringed and a click goes to it.

use rune::{Color, El, Icon, ScrollSpec, TextStyle, col, el, icon, memo, row, text};

use super::compose::button;
use super::dates;
use super::html_view::{self, Ink};
use super::style::{self, BODY, BODY_BOLD, Palette, READER, SECTION, SMALL};
use crate::app::App;
use crate::jmap::{Addr, Summary, format_addrs};

const NAME: TextStyle = TextStyle::new(14.0, 700.0);
const EMPTY: TextStyle = TextStyle::new(20.0, 500.0);
const SUBJECT: TextStyle = TextStyle::new(18.0, 700.0);
const MAP_NAME: TextStyle = TextStyle::new(11.0, 600.0);
const MAP_TIME: TextStyle = TextStyle::new(10.5, 400.0);
const MAP_WIDTH: f32 = 156.0;
/// The space around and between message cards, and their corners.
pub const CARD_GAP: f32 = 16.0;
const CARD_RADIUS: f32 = 10.0;
/// Inside a card.
const PAD: f32 = 20.0;
/// The widest a message's body column gets, its padding included.
const BODY_MAX: f32 = 900.0;
/// Around a message shown on white in dark mode.
const BACKING_PAD: f32 = 16.0;

/// The width a message's HTML is laid out at, in a reader pane `reader` wide: alone or in a
/// conversation (which has the map beside it), on a white backing or not. Worked out ahead
/// rather than measured, so a page is laid out at the width it shows at before it first
/// shows, and doesn't open wide then narrow. Keep in step with [`view`] and [`message`].
pub fn page_width(reader: f32, in_conversation: bool, backed: bool) -> f32 {
    let map = if in_conversation { MAP_WIDTH + 1.0 } else { 0.0 };
    let card = reader - map - 2.0 * CARD_GAP;
    let body = card.min(BODY_MAX) - 2.0 * PAD;
    body - if backed { 2.0 * BACKING_PAD } else { 0.0 }
}

/// `height` is the pane's, last frame: a conversation leaves that much room below its last
/// message, so moving with j/k can bring any message to the top.
pub fn view(s: &App, height: f32, p: Palette) -> El {
    let pane = col().key("reader").grow(1.0).min_w(0.0).h_full().fill(p.reader_bg).on_drawn_layout(|s: &mut App, r: [f32; 4]| {
        if (r[2] - s.reader_width).abs() >= 1.0 {
            s.reader_width = r[2];
        }
    });
    if !s.marked.is_empty() {
        let n = s.marked.len();
        let noun = match (s.settings.conversations, n == 1) {
            (true, true) => "Conversation",
            (true, false) => "Conversations",
            (false, true) => "Message",
            (false, false) => "Messages",
        };
        let hint = if s.settings.vim { "d Delete · a Archive · u Read · s Flag · Esc Clear" } else { "Delete, archive or flag them from the toolbar" };
        return pane.center().gap(8.0)
            .child(text(format!("{n} {noun} Marked")).font(EMPTY).color(p.secondary))
            .child(text(hint).font(SMALL).color(p.tertiary));
    }
    let Some(row_msg) = s.selected_message() else {
        return pane.center().child(text("No Message Selected").font(EMPTY).color(p.tertiary));
    };
    let members = s.conversation();
    let colors = Colors::new(&members, p);
    let single = members.len() == 1;
    let focus = s.focused_uid();
    // Each message is a card on the reader's tint, CARD_GAP apart and from the edges.
    let mut page = col().w_full().px(CARD_GAP).pt(CARD_GAP).gap(CARD_GAP * 0.75);
    if !single {
        let unread = members.iter().filter(|m| !m.seen).count();
        let count = if unread > 0 { format!("{} messages, {unread} unread", members.len()) } else { format!("{} messages", members.len()) };
        page = page.child(row().key("thread-head").w_full().px(4.0).pt(2.0).gap(12.0).items_start()
            .child(text(subject(row_msg)).font(SUBJECT).color(p.text).wrap().grow(1.0).min_w(0.0)
                .selectable(true).selection_color(p.text_selection))
            .child(text(count).font(SMALL).color(p.secondary).pt(4.0)));
    }
    for m in &members {
        let open = single || s.expanded.contains(&m.uid);
        let inner = if open { message(s, m, single, &colors, p) } else { collapsed(m, &colors, p) };
        // The highlighted message's card is ringed in the accent colour.
        let focused = !single && focus.as_deref() == Some(&m.uid);
        page = page.child(col().key(App::message_key(&m.uid)).w_full().radius(CARD_RADIUS).clip_rounded(CARD_RADIUS)
            .fill(p.card)
            .border(if focused { 1.5 } else { 1.0 }, if focused { p.accent } else { p.separator })
            .shadow(1.0, 3.0, Color::BLACK.alpha(if p.dark { 0.25 } else { 0.05 }))
            .child(inner));
    }
    if !single {
        page = page.child(el().key("thread-room").w_full().h((height - 120.0).max(0.0)));
    }
    let scroll = col().key(format!("read-{}", row_msg.uid)).grow(1.0).min_w(0.0).h_full().min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(page.pb(40.0));
    if single {
        return pane.child(scroll);
    }
    pane.child(row().w_full().grow(1.0).min_h(0.0)
        .child(scroll)
        .child(el().w(1.0).h_full().fill(p.separator))
        .child(map(s, &members, focus.as_deref(), &colors, p)))
}

fn subject(m: &Summary) -> String {
    if m.subject.is_empty() { "(no subject)".to_owned() } else { m.subject.clone() }
}

fn hash(s: &str) -> u32 {
    s.to_lowercase().bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193))
}

/// Colours for a conversation's people: the first takes a hue from their address and each
/// newcomer the next golden-angle step (137.5°), so two people in one conversation never
/// share a colour.
struct Colors {
    hues: Vec<(String, f32)>,
    dark: bool,
}

impl Colors {
    fn new(members: &[&Summary], p: Palette) -> Colors {
        let mut hues: Vec<(String, f32)> = Vec::new();
        let base = members.first().and_then(|m| m.from.first()).map_or(0, |a| hash(&a.email) % 360) as f32;
        for m in members.iter().rev() {
            let Some(a) = m.from.first() else { continue };
            let email = a.email.to_lowercase();
            if !hues.iter().any(|(e, _)| *e == email) {
                let hue = (base + hues.len() as f32 * 137.508) % 360.0;
                hues.push((email, hue));
            }
        }
        Colors { hues, dark: p.dark }
    }

    fn of(&self, from: Option<&Addr>) -> Color {
        let email = from.map_or(String::new(), |a| a.email.to_lowercase());
        let hue = self.hues.iter().find(|(e, _)| *e == email).map_or(hash(&email) as f32 % 360.0, |(_, h)| *h);
        rune::oklch(if self.dark { 0.6 } else { 0.64 }, 0.12, hue)
    }
}

/// The conversation map: one block per message, in reader order.
fn map(s: &App, members: &[&Summary], focus: Option<&str>, colors: &Colors, p: Palette) -> El {
    let mut list = col().w_full().px(8.0).pt(12.0).pb(16.0).gap(4.0)
        .child(row().w_full().px(6.0).pb(4.0)
            .child(text(format!("{} MESSAGES", members.len())).font(SECTION).color(p.secondary)));
    for m in members {
        let open = s.expanded.contains(&m.uid);
        let color = colors.of(m.from.first());
        let name = m.from.first().map_or("?", Addr::display);
        let first = name.split_whitespace().next().unwrap_or(name).to_owned();
        // Blocks are as tall as the text is long (body when loaded, else preview), so a long
        // message stands out the way a long function does in an editor's minimap.
        let len = s.bodies.get(&m.uid).map_or(m.preview.len() * 3, |b| b.text.len());
        let lines = (len / 140).clamp(1, if open { 14 } else { 3 });
        let mut bars = col().w_full().gap(2.0).pt(4.0);
        let seed = m.uid.bytes().fold(7u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
        for i in 0..lines {
            let w = 0.45 + ((seed.rotate_left(i as u32 * 5) % 55) as f32 / 100.0);
            bars = bars.child(el().w(w * (MAP_WIDTH - 40.0)).h(3.0).radius(1.5).fill(color.alpha(if open { 0.55 } else { 0.25 })));
        }
        let focused = focus == Some(m.uid.as_str());
        let uid = m.uid.clone();
        let mut head = row().w_full().gap(5.0).center_y()
            .child(el().size(7.0, 7.0).radius(3.5).fill(color))
            .child(text(first).font(MAP_NAME).color(if m.seen { p.secondary } else { p.text }).grow(1.0).min_w(0.0).truncate());
        if !m.seen {
            head = head.child(el().size(6.0, 6.0).radius(3.0).fill(p.accent));
        }
        let when = text(dates::compact(&m.received_at)).font(MAP_TIME).color(p.tertiary).pl(12.0).truncate();
        list = list.child(col().key(format!("map-{}", m.uid)).w_full().px(6.0).py(6.0).radius(6.0)
            .fill(if focused { p.accent.alpha(0.12) } else { Color::TRANSPARENT })
            .border(1.0, if focused { p.accent.alpha(0.6) } else { Color::TRANSPARENT })
            .hover(p.hover)
            .help(format!("{name} · {}", dates::with_time(&m.received_at)))
            .on_press_tx(move |s: &mut App, tx| {
                if !s.expanded.contains(&uid) {
                    s.toggle_expanded(&uid);
                }
                s.pane = crate::app::Pane::Thread;
                s.thread_focus = Some(uid.clone());
                tx.scroll_key_to(App::message_key(&uid), CARD_GAP);
            })
            .child(head)
            .child(when)
            .child(bars));
    }
    col().key("thread-map").w(MAP_WIDTH).h_full().min_h(0.0).fill(p.sidebar.alpha(0.5))
        .scroll(ScrollSpec { stick_bottom: false })
        .child(list)
}

/// A message that isn't open, drawn like a list row: sender and time, subject, and two
/// lines of preview. A click opens it.
fn collapsed(m: &Summary, colors: &Colors, p: Palette) -> El {
    let uid = m.uid.clone();
    let from = m.from.first();
    let mut top = row().w_full().gap(6.0).center_y()
        .child(text(from.map_or("(no sender)", Addr::display).to_owned()).font(BODY_BOLD).color(p.text).grow(1.0).min_w(0.0).truncate());
    if !m.seen {
        top = top.child(el().size(8.0, 8.0).radius(4.0).fill(p.accent));
    }
    if m.flagged {
        top = top.child(icon(&style::FLAG, 11.0).color(p.flag));
    }
    top = top.child(text(dates::with_time(&m.received_at)).font(SMALL).color(p.secondary));
    row().key(format!("msg-line-{}", m.uid)).w_full().px(PAD).py(12.0).gap(12.0).items_start()
        .hover(p.hover)
        .on_press_tx(move |s: &mut App, tx| {
            s.toggle_expanded(&uid);
            s.focus_message(&uid, tx);
        })
        .child(avatar(from, colors.of(from), 32.0))
        .child(col().grow(1.0).min_w(0.0).gap(2.0)
            .child(top)
            .child(text(subject(m)).font(BODY).color(p.text).truncate())
            .child(text(m.preview.clone()).font(SMALL).color(p.secondary).wrap().max_lines(2)))
}

fn message(s: &App, m: &Summary, single: bool, colors: &Colors, p: Palette) -> El {
    let body = s.bodies.get(&m.uid);
    let from = m.from.first();
    let when = if single { dates::long(&m.received_at) } else { dates::with_time(&m.received_at) };
    let mut details = col().grow(1.0).min_w(0.0).gap(3.0)
        .child(row().w_full().gap(12.0)
            .child(text(from.map_or("(no sender)", Addr::display).to_owned()).font(NAME).color(p.text)
                .grow(1.0).min_w(0.0).truncate().selectable(true).selection_color(p.text_selection))
            .child(text(when).font(SMALL).color(p.secondary)));
    if single {
        details = details.child(text(subject(m)).font(BODY_BOLD).color(p.text).wrap()
            .selectable(true).selection_color(p.text_selection));
    }
    details = details.child(line("To:", &format_addrs(&m.to), p));
    if let Some(b) = body.filter(|b| !b.cc.is_empty()) {
        details = details.child(line("Cc:", &format_addrs(&b.cc), p));
    }
    let html = body.and_then(|b| b.html.as_ref()).filter(|_| s.settings.render_html);
    // In dark mode, any message can be read on white; one that sets its own colours already is.
    let white = p.dark && s.on_white.contains(&m.uid);
    let paper_toggle = (p.dark && body.is_some() && !html.is_some_and(|d| d.colored)).then(|| {
        let uid = m.uid.clone();
        let (i, help) = if white { (&style::MOON, "Show in Dark Colours") } else { (&style::SUN, "Show on White") };
        mini(i, help, p).key(format!("paper-{}", m.uid)).on_press(move |s: &mut App| s.toggle_white(&uid))
    });
    let uid = m.uid.clone();
    let mut header = row().w_full().px(PAD).pt(if single { PAD } else { 14.0 }).pb(12.0).gap(12.0).items_start()
        .child(avatar(from, colors.of(from), 40.0))
        .child(details);
    if single && let Some(t) = paper_toggle.clone() {
        header = header.child(t);
    }
    if !single {
        // In a conversation, each message answers for itself; the header folds it away.
        let (r, a, f) = (uid.clone(), uid.clone(), uid.clone());
        header = header
            .child(row().gap(2.0)
                .children(paper_toggle)
                .child(mini(&style::REPLY, "Reply (r)", p).on_press(move |s: &mut App| s.reply_to(Some(&r), false)))
                .child(mini(&style::REPLY_ALL, "Reply All (R)", p).on_press(move |s: &mut App| s.reply_to(Some(&a), true)))
                .child(mini(&style::FORWARD, "Forward (f)", p).on_press(move |s: &mut App| s.forward_message(Some(&f)))))
            .on_press_tx(move |s: &mut App, tx| {
                s.toggle_expanded(&uid);
                s.focus_message(&uid, tx);
            });
    }

    let mut card = col().key(format!("msg-card-{}", m.uid)).w_full().child(header);
    let remote_blocked = html.is_some_and(|d| d.remote_images)
        && !s.settings.remote_images
        && !s.remote_allowed.contains(&m.uid);
    if let Some(u) = body.and_then(|b| b.unsubscribe.as_ref()) {
        card = card.child(unsubscribe_bar(s, m, u, p));
    }
    if let Some(bar) = privacy_bar(m, html, remote_blocked, p) {
        card = card.child(bar);
    }
    let content = match (body, html) {
        (None, _) => match s.body_failed.get(&m.uid) {
            Some(error) => {
                let uid = m.uid.clone();
                col().w_full().gap(8.0).items_start()
                    .child(text("This message didn't load.").font(BODY).color(p.text))
                    .child(text(error.clone()).font(SMALL).color(p.secondary).wrap())
                    .child(button("Try Again", p.text, p.field, p).on_press(move |s: &mut App| s.retry_body(&uid)))
            }
            // What the list already has shows at once; the rest follows.
            None if !m.preview.trim().is_empty() => col().w_full().gap(10.0)
                .child(html_view::plain(m.preview.trim(), READER, p.text, p.accent, p.text_selection))
                .child(text("Loading the rest…").font(SMALL).color(p.tertiary)),
            None => text("Loading…").font(BODY).color(p.secondary),
        },
        // Its HTML couldn't be laid out: the text, rather than nothing.
        (Some(b), Some(_)) if s.pages.failed.contains(&m.uid) => memo(format!("plain-{}", m.uid), (p.dark, b.text.len()), || {
            html_view::plain(&b.text, READER, p.text, p.accent, p.text_selection)
        }),
        (Some(_), Some(doc)) => {
            let (_, paper) = Ink::for_doc(doc, white, p);
            let drawn = super::page_view::view(s, &m.uid);
            if paper && p.dark {
                col().w_full().padding(BACKING_PAD).radius(8.0).fill(Color::WHITE).child(drawn)
            } else {
                drawn
            }
        }
        (Some(b), None) if white => memo(format!("plain-white-{}", m.uid), b.text.len(), || {
            let ink = Ink::paper();
            col().w_full().padding(16.0).radius(8.0).fill(Color::WHITE)
                .child(html_view::plain(&b.text, READER, ink.text, ink.link, ink.selection))
        }),
        // Kept between frames like the HTML above: it copies and scans the whole text for links.
        (Some(b), None) => memo(format!("plain-{}", m.uid), (p.dark, b.text.len()), || {
            html_view::plain(&b.text, READER, p.text, p.accent, p.text_selection)
        }),
    };
    if single {
        card = card.child(row().w_full().px(PAD).child(el().grow(1.0).h(1.0).fill(p.separator)));
    }
    card = card.child(col().w_full().max_w(BODY_MAX).px(PAD).pt(if single { 18.0 } else { 4.0 }).pb(PAD).child(content));
    match body.filter(|b| !b.files.is_empty()) {
        Some(b) => card.child(files(m, &b.files, p)),
        None => card,
    }
}

/// The message's attachments: press one to open it, or its arrow to save it somewhere.
fn files(m: &Summary, list: &[crate::jmap::FileInfo], p: Palette) -> El {
    let mut strip = row().key(format!("files-{}", m.uid)).w_full().px(PAD).pb(PAD).gap(8.0).flex_wrap();
    for (i, f) in list.iter().enumerate() {
        let (open, save) = (m.uid.clone(), m.uid.clone());
        strip = strip.child(
            row().key(format!("file-{}-{i}", m.uid)).h(36.0).max_w(280.0).pl(10.0).pr(4.0).gap(8.0).center_y()
                .radius(8.0).fill(p.field).hover(p.hover).help(format!("Open {}", f.name))
                .on_press(move |s: &mut App| s.open_attachment(&open, i))
                .child(icon(if f.mime.starts_with("image/") { &style::IMAGE } else { &style::CLIP }, 14.0).color(p.secondary))
                .child(col().shrink().min_w(0.0)
                    .child(text(f.name.clone()).font(SMALL).color(p.text).truncate())
                    .child(text(super::sheets::size(f.size)).font(style::COUNT).color(p.secondary)))
                .child(el().key(format!("file-save-{}-{i}", m.uid)).size(26.0, 26.0).radius(6.0).center().hover(p.hover)
                    .help("Save…")
                    .on_press_tx(move |s: &mut App, tx| s.save_attachment(&save, i, tx))
                    .child(icon(&style::DOWNLOAD, 13.0).color(p.secondary))),
        );
    }
    strip
}

/// "This message is from a mailing list", with Unsubscribe; asked once, it asks to confirm
/// and says how it will happen.
fn unsubscribe_bar(s: &App, m: &Summary, u: &crate::privacy::Unsubscribe, p: Palette) -> El {
    let list = u.list.clone().or_else(|| m.from.first().map(|a| a.display().to_owned())).unwrap_or_default();
    let bar = row().key(format!("unsub-{}", m.uid)).w_full().px(PAD).py(7.0).gap(10.0).center_y()
        .child(icon(&style::UNSUBSCRIBE, 14.0).color(p.secondary));
    let target = s.unsubscribe_target().is_some_and(|(t, _)| t.uid == m.uid);
    if s.unsubscribed.contains(&m.uid) {
        return bar.child(text(format!("Unsubscribed from {list}")).font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).truncate());
    }
    if s.unsubscribe_confirm.as_ref() == Some(&m.uid) {
        let key = if s.settings.vim { "X" } else { "Ctrl+Shift+X" };
        return bar.fill(p.accent.alpha(0.08))
            .child(text(format!("Unsubscribe from {list} by {}?", u.method())).font(SMALL).color(p.text).grow(1.0).min_w(0.0).wrap())
            .child(button("Cancel", p.text, p.field, p).help("Esc").on_press(|s: &mut App| {
                s.unsubscribe_confirm = None;
            }))
            .child(button("Unsubscribe", p.on_selection, p.accent, p).help(format!("{key} again"))
                .on_press_tx(|s: &mut App, tx| s.unsubscribe(tx)));
    }
    let uid = m.uid.clone();
    bar.child(text(format!("This message is from a mailing list: {list}")).font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).truncate())
        .child(button("Unsubscribe", p.accent, p.field, p).help("Unsubscribe (Ctrl+Shift+X, or X)")
            .on_press_tx(move |s: &mut App, tx| {
                if !target {
                    // In a conversation, aim at this message first.
                    s.thread_focus = Some(uid.clone());
                }
                s.unsubscribe(tx);
            }))
}

/// What was kept from the sender: trackers blocked, remote images held back, tracked links.
fn privacy_bar(m: &Summary, html: Option<&crate::html::Doc>, remote_blocked: bool, p: Palette) -> Option<El> {
    let doc = html?;
    let mut parts = Vec::new();
    if !doc.trackers.is_empty() {
        let n = doc.trackers.len();
        parts.push(format!("Blocked {} ({})", if n == 1 { "a tracker".to_owned() } else { format!("{n} trackers") }, doc.trackers.join(", ")));
    }
    if remote_blocked {
        parts.push("Remote images are off".to_owned());
    }
    if !doc.tracked_links.is_empty() {
        parts.push(format!("Links are tracked by {}", doc.tracked_links.join(", ")));
    }
    if parts.is_empty() {
        return None;
    }
    let mut bar = row().key(format!("privacy-{}", m.uid)).w_full().px(PAD).py(7.0).gap(10.0).center_y().fill(p.field)
        .help("Trackers never load. Links are cleaned of tracking when opened; tracked links still pass through the sender's tracker.")
        .child(icon(&style::SHIELD, 14.0).color(p.accent))
        .child(text(parts.join(" · ")).font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).wrap());
    if remote_blocked {
        let id = m.uid.clone();
        bar = bar.child(button("Load Remote Content", p.text, p.content, p).help("Load Remote Content (i). Trackers stay blocked.")
            .on_press(move |s: &mut App| s.allow_remote(id.clone())));
    }
    Some(bar)
}

fn mini(i: &'static Icon, help: &str, p: Palette) -> El {
    el().size(26.0, 24.0).radius(6.0).center().hover(p.hover).help(help)
        .child(icon(i, 14.0).color(p.secondary))
}

fn line(label: &str, value: &str, p: Palette) -> El {
    row().w_full().gap(4.0)
        .child(text(label).font(SMALL).color(p.secondary))
        .child(text(value).font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).truncate()
            .selectable(true).selection_color(p.text_selection))
}

/// A circle with the sender's initials in their colour.
fn avatar(from: Option<&Addr>, fill: Color, size: f32) -> El {
    let name = from.map_or("?", Addr::display);
    let initials: String =
        name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
    el().size(size, size).radius(size / 2.0).fill(fill).center()
        .child(text(initials).font(TextStyle::new(size * 0.375, 600.0)).color(Color::WHITE))
}
