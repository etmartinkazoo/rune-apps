//! The sidebar: Messages, Calls and Contacts, a search field, and the list of the tab chosen.

use rune::{Anim, Color, Drag, El, GrowthSpec, Phase, ScrollSpec, Tx, col, el, icon, row, text};

use super::field::{input, platform_key};
use super::style::{self, BODY, BODY_BOLD, Palette, SMALL};
use super::{avatar, swipeable, tool};
use crate::actions::{Focus, Target};
use crate::app::{App, MAX_PINS, PIN_COLUMNS, Pane, Tab, Thread, when};
use crate::vcard::CallKind;

pub const WIDTH: f32 = 320.0;
const ROW: f32 = 64.0;

pub fn view(s: &App, p: Palette) -> El {
    let tabs = row().w_full().h(30.0).padding(2.0).gap(2.0).radius(8.0).fill(p.field)
        .child(tab(s, Tab::Messages, "Messages", p))
        .child(tab(s, Tab::Calls, "Calls", p))
        .child(tab(s, Tab::Contacts, "Contacts", p));
    let phone = s.link.phone.as_ref().map_or("No phone".to_owned(), |d| d.name.clone());
    let (dot, state) = match (s.link.calls, s.link.messages) {
        (true, true) => (*style::GREEN, "Connected"),
        (true, false) | (false, true) => (*style::GREEN, "Partly connected"),
        _ if s.connecting => (p.tertiary, "Connecting…"),
        _ if s.link.phone.is_none() => (p.tertiary, "No phone"),
        _ => (*style::RED, "Not connected"),
    };
    let head = row().w_full().gap(8.0).center_y()
        .child(el().size(8.0, 8.0).radius(4.0).fill(dot))
        .child(col().grow(1.0).min_w(0.0)
            .child(text(phone).font(BODY_BOLD).color(p.text).truncate())
            .child(text(state).font(SMALL).color(p.secondary)))
        .child(tool(&style::BLUETOOTH, "Phones", p).key("phones").on_press(|s: &mut App| s.open_setup()))
        .child(tool(&style::KEYPAD, "Keypad", p).key("keypad").on_press(|s: &mut App| {
            s.tab = Tab::Calls;
            s.pane = Pane::Keypad;
        }))
        .child(tool(&style::COMPOSE, "New Message (Ctrl+N)", p).key("compose").on_press(|s: &mut App| s.compose()));
    let search = row().w_full().h(28.0).px(8.0).gap(6.0).center_y().radius(7.0).fill(p.field)
        .child(icon(&style::SEARCH, 13.0).color(p.secondary))
        .child(input(&s.search, BODY, p).key("search").single_line().grow(1.0).min_w(0.0).placeholder("Search", p.tertiary)
            .on_edit(|s: &mut App| &mut s.search)
            .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut Tx| platform_key(&mut s.search, e)))
        .on_press(|s: &mut App| s.search.focused = true);
    let list = match s.tab {
        Tab::Messages => threads(s, p),
        Tab::Calls => recents(s, p),
        Tab::Contacts => contacts(s, p),
    };
    col().key("sidebar").w(WIDTH).h_full().fill(p.sidebar).pt(14.0).gap(10.0)
        .child(col().w_full().px(12.0).gap(10.0).child(head).child(tabs).child(search))
        .child(list)
}

fn tab(s: &App, t: Tab, label: &str, p: Palette) -> El {
    let on = s.tab == t;
    row().key(format!("tab-{}", label.to_lowercase())).grow(1.0).h(26.0).center().radius(6.0)
        .fill(if on { p.content } else { Color::TRANSPARENT })
        .hover(p.hover)
        .on_press(move |s: &mut App| s.tab = t)
        .child(text(label.to_owned()).font(SMALL).color(if on { p.text } else { p.secondary }))
}

fn scroller(key: &str, count: usize, build: impl Fn(&App, usize) -> El + 'static, id: impl Fn(&App, usize) -> String + 'static) -> El {
    let rows = col().key(format!("{key}-rows")).w_full().px(8.0).growth(GrowthSpec::default())
        .virtual_rows(count, ROW, move |s: &App, i| id(s, i).into(), move |s: &App, i| build(s, i));
    col().key(key.to_owned()).w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(rows)
}

fn empty(note: &str, p: Palette) -> El {
    col().w_full().grow(1.0).center().px(24.0).child(text(note.to_owned()).font(BODY).color(p.secondary).wrap().align(rune::Align::Center))
}

fn item(on: bool, key: String, p: Palette) -> El {
    row().key(key).w_full().h(ROW - 4.0).my(2.0).px(8.0).gap(10.0).center_y().radius(8.0)
        .fill(if on { p.accent } else { Color::TRANSPARENT })
        .hover(if on { Color::TRANSPARENT } else { p.hover })
}

fn threads(s: &App, p: Palette) -> El {
    // Pins stay out of a search: what matches it is in the list.
    let pins = if s.search.text.trim().is_empty() { s.pinned_threads() } else { Vec::new() };
    let count = s.visible_threads().len();
    if count == 0 && pins.is_empty() {
        return empty(if s.link.messages { "No messages" } else { "Messages appear once the phone allows access to them" }, p);
    }
    let list = scroller(
        "threads",
        count,
        move |s: &App, i| {
            let Some(t) = s.visible_threads().get(i).copied() else { return el() };
            let on = s.pane == Pane::Thread(t.key.clone());
            let (ink, muted) = if on { (p.on_selection, p.on_selection.alpha(0.8)) } else { (p.text, p.secondary) };
            let name = if t.name.is_empty() { s.who(&t.number) } else { t.name.clone() };
            let last = t.last();
            let preview = last.map_or(String::new(), |m| if m.text.is_empty() && m.mms { "Photo".into() } else { m.text.replace('\n', " ") });
            let key = t.key.clone();
            let pin_key = t.key.clone();
            let menu_key = t.key.clone();
            let full = s.settings.pinned.len() >= MAX_PINS;
            let picked = s.picked.threads.contains(&t.key);
            let cursor = s.focus == Focus::List && s.cursor.as_deref() == Some(t.key.as_str());
            let target = Target::Thread(t.key.clone());
            let swiping = s.swiped_by(&target) != 0.0;
            let (drag_target, wheel_target) = (target.clone(), target.clone());
            let mut row_el = item(on, format!("thread-{}", t.key), p);
            if picked && !on {
                row_el = row_el.fill(p.accent.alpha(0.2));
            } else if swiping && !on {
                row_el = row_el.fill(p.sidebar);
            }
            if cursor {
                row_el = row_el.outline(1.5, 0.0, p.accent);
            }
            let row_el = row_el
                .hover_group()
                .on_press_tx(move |s: &mut App, tx: &mut Tx| s.click_thread(&key, tx))
                .on_drag(move |s: &mut App, d: &mut Drag, tx: &mut Tx| match d.phase {
                    Phase::Down => s.press_down(),
                    Phase::Move => {
                        tx.animated();
                        s.drag_swipe(drag_target.clone(), d.dx, d.dy);
                    }
                    Phase::Up => s.end_swipe(d.cancelled(), tx),
                })
                .on_wheel(move |s: &mut App, e: rune::WheelEvent, tx: &mut Tx| s.wheel_swipe(wheel_target.clone(), &e, tx))
                .on_secondary_press(move |s: &mut App, _, tx: &mut Tx| s.toggle_pin(&menu_key, tx))
                .child(el().size(16.0, 16.0).center().child(if picked {
                    el().size(16.0, 16.0).radius(8.0).center().fill(if on { ink } else { p.accent })
                        .child(icon(&style::CHECK, 11.0).color(if on { p.accent } else { Color::WHITE }))
                } else {
                    el().size(8.0, 8.0).radius(4.0).fill(if t.unread > 0 { if on { ink } else { p.accent } } else { Color::TRANSPARENT })
                }))
                .child(avatar(&name, 40.0))
                .child(col().grow(1.0).min_w(0.0)
                    .child(row().w_full().gap(6.0)
                        .child(text(name).font(BODY_BOLD).color(ink).grow(1.0).min_w(0.0).truncate())
                        .child(text(last.map_or(String::new(), |m| when(&m.at))).font(SMALL).color(muted)))
                    .child(text(preview).font(SMALL).color(muted).max_lines(2).wrap()))
                .child(el().key(format!("pin-{}", t.key)).size(24.0, 24.0).radius(12.0).center()
                    .fill(if on { p.on_selection.alpha(0.2) } else { p.field })
                    .hover(p.hover)
                    .group_opacity(0.0, if full { 0.35 } else { 1.0 })
                    .help(if full { "Six are pinned: unpin one first" } else { "Pin to the top (or right-click)" })
                    .on_press_tx(move |s: &mut App, tx: &mut Tx| s.toggle_pin(&pin_key, tx))
                    .child(icon(&style::PIN, 13.0).color(ink)));
            swipeable(s, target, row_el, 8.0, 2.0, p)
        },
        |s: &App, i| s.visible_threads().get(i).map_or(String::new(), |t| t.key.clone()),
    );
    let bar = (!s.picked.threads.is_empty()).then(|| picked_bar(s, p));
    if pins.is_empty() && bar.is_none() {
        return list;
    }
    let mut c = col().key("threads-pinned").w_full().grow(1.0).min_h(0.0);
    if let Some(bar) = bar {
        c = c.child(bar);
    }
    if pins.is_empty() {
        return c.child(list);
    }
    c.child(pinned(s, &pins, p))
        .child(row().w_full().px(16.0).child(el().w_full().h(1.0).fill(p.separator)))
        .child(list)
}

/// What can be done with the conversations picked.
fn picked_bar(s: &App, p: Palette) -> El {
    let n = s.picked.threads.len();
    let unread = s.picked.threads.iter().any(|k| s.thread(k).is_some_and(|t| t.unread > 0));
    let pinned = s.picked.threads.iter().all(|k| s.is_pinned(k));
    row().key("picked-threads").w_full().px(12.0).pb(8.0).gap(4.0).center_y()
        .child(text(format!("{n} selected")).font(BODY_BOLD).color(p.text).grow(1.0))
        .child(small_tool(&style::MAIL, if unread { "Mark read (u)" } else { "Mark unread (u)" }, p.secondary, p).key("picked-read")
            .on_press_tx(|s: &mut App, tx: &mut Tx| s.toggle_read(&s.picked_threads(), tx)))
        .child(small_tool(&style::PIN, if pinned { "Unpin (p)" } else { "Pin (p)" }, p.secondary, p).key("picked-pin")
            .on_press_tx(|s: &mut App, tx: &mut Tx| s.toggle_pins(&s.picked_threads(), tx)))
        .child(small_tool(&style::TRASH, "Delete (dd)", *style::RED, p).key("picked-delete")
            .on_press_tx(|s: &mut App, tx: &mut Tx| s.ask_delete(&s.picked_threads(), tx)))
        .child(small_tool(&style::CLOSE, "Let go of them (Escape)", p.secondary, p).key("picked-clear")
            .on_press(|s: &mut App| s.clear_picks()))
}

pub fn small_tool(i: &'static rune::Icon, help: &str, ink: Color, p: Palette) -> El {
    el().size(28.0, 26.0).radius(6.0).center().hover(p.hover).help(help.to_owned()).child(icon(i, 15.0).color(ink))
}

/// The pinned conversations, three to a row: a big avatar and the name, the unread dot beside
/// it, and on hover a badge to unpin.
fn pinned(s: &App, pins: &[&Thread], p: Palette) -> El {
    const AVATAR: f32 = 54.0;
    // The sidebar less its padding and the gaps, in three.
    const TILE: f32 = (WIDTH - 16.0 - 4.0) / 3.0;
    const GAP: f32 = 2.0;
    let count = pins.len();
    let drag = s.pin_drag;
    let mut grid = col().w_full().px(8.0).pb(6.0).gap(GAP);
    for (r, chunk) in pins.chunks(PIN_COLUMNS).enumerate() {
        let mut line = row().w_full().gap(GAP);
        for (c, t) in chunk.iter().enumerate() {
            let index = r * PIN_COLUMNS + c;
            let lifted = drag.from == Some(index);
            let on = s.pane == Pane::Thread(t.key.clone());
            let ink = if on { p.on_selection } else { p.text };
            let name = if t.name.is_empty() { s.who(&t.number) } else { t.name.clone() };
            let first = name.split_whitespace().next().unwrap_or_default().to_owned();
            let (open, unpin, menu) = (t.key.clone(), t.key.clone(), t.key.clone());
            let face = el().size(AVATAR + 8.0, AVATAR + 4.0)
                .child(avatar(&name, AVATAR).absolute(4.0, 2.0))
                .child(el().key(format!("unpin-{}", t.key)).absolute(0.0, 0.0).size(20.0, 20.0).radius(10.0).center()
                    .fill(p.tertiary).group_opacity(0.0, 1.0).help("Unpin")
                    .on_press_tx(move |s: &mut App, tx: &mut Tx| s.toggle_pin(&unpin, tx))
                    .child(icon(&style::MINUS, 12.0).color(Color::WHITE)));
            let label = row().w_full().gap(4.0).center()
                .child(el().size(7.0, 7.0).radius(3.5).fill(if t.unread > 0 { if on { ink } else { p.accent } } else { Color::TRANSPARENT }))
                .child(text(first).font(if t.unread > 0 { BODY_BOLD } else { SMALL }).color(ink).min_w(0.0).truncate())
                .child(el().size(7.0, 7.0));
            let (x, y) = drag.shift(index);
            let picked = s.picked.threads.contains(&t.key);
            let cursor = s.focus == Focus::List && s.cursor.as_deref() == Some(t.key.as_str());
            let mut tile = col().key(format!("pinned-{}", t.key)).w(TILE).py(6.0).px(4.0).gap(4.0).items_center().radius(10.0)
                .fill(if on { p.accent } else if lifted { p.field } else if picked { p.accent.alpha(0.2) } else { Color::TRANSPARENT })
                .hover(if on || lifted { Color::TRANSPARENT } else { p.hover })
                .offset(x, y);
            if cursor {
                tile = tile.outline(1.5, 0.0, p.accent);
            }
            if lifted {
                // Over its neighbours, with the pointer exactly; they spring aside around it.
                tile = tile.raise().offset_motion(Anim::INSTANT).shadow(6.0, 18.0, Color::BLACK.alpha(0.3));
            }
            line = line.child(tile
                .hover_group()
                .help(if drag.from.is_some() { String::new() } else { format!("{name} (drag to reorder)") })
                .on_drag(move |s: &mut App, d: &mut Drag, tx: &mut Tx| match d.phase {
                    Phase::Down => s.pin_drag.pressed(),
                    Phase::Move => {
                        tx.animated();
                        s.drag_pin(index, count, d.dx, d.dy, (d.w, d.h), GAP);
                    }
                    Phase::Up => s.drop_pin(d.cancelled()),
                })
                .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    let m = tx.mods();
                    if m.ctrl || m.cmd || m.shift { s.click_thread(&open, tx) } else { s.press_pin(&open) }
                })
                .on_secondary_press(move |s: &mut App, _, tx: &mut Tx| s.toggle_pin(&menu, tx))
                .child(face)
                .child(label));
        }
        grid = grid.child(line);
    }
    grid
}

fn recents(s: &App, p: Palette) -> El {
    if s.recents.is_empty() {
        return empty(if s.link.contacts { "No recent calls" } else { "Recent calls appear once the phone allows access to contacts" }, p);
    }
    scroller(
        "recents",
        s.recents.len(),
        move |s: &App, i| {
            let Some(c) = s.recents.get(i) else { return el() };
            let name = if c.name.is_empty() { s.who(&c.number) } else { c.name.clone() };
            let (glyph, colour, what) = match c.kind {
                CallKind::Incoming => (&*style::INCOMING, p.secondary, "Incoming"),
                CallKind::Outgoing => (&*style::OUTGOING, p.secondary, "Outgoing"),
                CallKind::Missed => (&*style::INCOMING, *style::RED, "Missed"),
            };
            let number = c.number.clone();
            item(false, format!("recent-{i}"), p)
                .help(format!("Call {}", crate::number::pretty(&c.number)))
                .on_press_tx(move |s: &mut App, tx: &mut Tx| s.call(&number, tx))
                .child(icon(glyph, 14.0).color(colour))
                .child(col().grow(1.0).min_w(0.0)
                    .child(text(name).font(BODY_BOLD).color(if c.kind == CallKind::Missed { *style::RED } else { p.text }).truncate())
                    .child(text(what).font(SMALL).color(p.secondary)))
                .child(text(when(&c.at)).font(SMALL).color(p.secondary))
        },
        |_, i| format!("recent-{i}"),
    )
}

fn contacts(s: &App, p: Palette) -> El {
    let count = s.visible_contacts().len();
    if count == 0 {
        return empty(if s.link.contacts { "No contacts" } else { "Contacts appear once the phone allows access to them" }, p);
    }
    scroller(
        "contacts",
        count,
        move |s: &App, i| {
            let Some(&(index, c)) = s.visible_contacts().get(i) else { return el() };
            let on = s.pane == Pane::Contact(index);
            let ink = if on { p.on_selection } else { p.text };
            item(on, format!("contact-{index}"), p)
                .on_press(move |s: &mut App| s.pane = Pane::Contact(index))
                .child(avatar(&c.name, 36.0))
                .child(text(c.name.clone()).font(BODY_BOLD).color(ink).grow(1.0).min_w(0.0).truncate())
        },
        |s: &App, i| s.visible_contacts().get(i).map_or(String::new(), |(index, _)| index.to_string()),
    )
}
