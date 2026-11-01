//! Apple Mail's layout: the sidebar runs the full height; a toolbar spans the message list and
//! the reader beside it.

pub mod compose;
pub mod dates;
pub mod field;
pub mod html_view;
pub mod page_view;
pub mod keys;
pub mod list;
pub mod reader;
pub mod sheets;
pub mod sidebar;
pub mod signin;
pub mod style;
pub mod today;

use rune::{Color, Cx, El, Icon, Tx, col, el, icon, row, text};

use crate::app::{App, Phase, Sheet};
use style::{BODY, Palette, SMALL, TITLE};

pub use keys::on_key;

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    s.view_env.set((cx.env().scale, cx.system_dark()));
    let p = Palette::new(s.settings.dark(cx.system_dark()));
    if s.phase != Phase::Ready {
        return signin::view(s, p);
    }
    let main = row().w_full().h_full().fill(p.window)
        .child(sidebar::view(s, p))
        .child(vline(p))
        .child(col().grow(1.0).min_w(0.0).h_full()
            .child(toolbar(s, cx.layout_size("toolbar").map_or(f32::MAX, |[w, _]| w), p))
            .child(el().w_full().h(1.0).fill(p.separator))
            .child(row().w_full().grow(1.0).min_h(0.0)
                .child(list::view(s, p))
                .child(vline(p))
                .child(reader::view(s, cx.layout_size("reader").map_or(0.0, |[_, h]| h), p))));
    let mut root = el().key("root").w_full().h_full().child(main);
    if let Some((status, _)) = &s.status {
        root = root.child(row().key("status").pin(None, None, Some(16.0), Some(16.0)).px(14.0).h(32.0).center_y()
            .radius(8.0).fill(if p.dark { Color::WHITE } else { Color::BLACK }.alpha(0.85))
            .child(text(status.clone()).font(BODY).color(if p.dark { Color::BLACK } else { Color::WHITE })));
    }
    if let Some(pill) = undo_pill(s, p) {
        root = root.child(pill);
    }
    if let Some(d) = &s.dragging {
        // The label that follows the pointer while dragging messages to a mailbox.
        root = root.child(row().key("drag-label").pin(Some(d.x + 14.0), Some(d.y + 10.0), None, None)
            .h(26.0).px(10.0).gap(6.0).center_y().radius(13.0).max_w(320.0)
            .fill(if d.over.is_some() { p.accent } else { p.secondary })
            .shadow(4.0, 14.0, Color::BLACK.alpha(0.25))
            .inert()
            .child(icon(&style::ENVELOPE, 13.0).color(Color::WHITE))
            .child(text(d.label.clone()).font(SMALL).color(Color::WHITE).truncate().min_w(0.0)));
    }
    match s.sheet {
        Sheet::None => root,
        Sheet::Compose => root.child(compose::view(s, p)).children(compose::suggestions(s, p)).children(compose::from_menu(s, p)),
        Sheet::Settings => root.child(sheets::settings(s, p)),
        Sheet::Shortcuts => root.child(sheets::shortcuts(s, p)),
        Sheet::AddAccount => root.child(signin::sheet(s, p)),
    }
}

/// The pill at the window's bottom right: a send counting down, or the last action, with Undo.
fn undo_pill(s: &App, p: Palette) -> Option<El> {
    let (label, key) = match (s.waiting_send(), &s.undo) {
        (Some((pending, secs)), _) => {
            let to = pending.mail.to.first().map_or("", |a| a.display()).to_owned();
            (format!("Sending to {to} in {secs}s"), "undo-send")
        }
        (None, Some(u)) => (u.label.clone(), "undo-action"),
        (None, None) => return None,
    };
    // In the theme's own colours: a light pill in light mode, a dark one in dark.
    Some(row().key(key).pin(None, None, Some(16.0), Some(16.0)).h(34.0).pl(14.0).pr(6.0).gap(10.0)
        .center_y().radius(17.0).fill(p.card).border(1.0, p.separator)
        .shadow(4.0, 14.0, Color::BLACK.alpha(if p.dark { 0.4 } else { 0.15 }))
        .child(text(label).font(BODY).color(p.text))
        .child(row().key("undo-button").h(24.0).px(10.0).radius(12.0).center().fill(p.accent).hover(p.hover)
            .help("Undo (Ctrl+Z)")
            .on_press_tx(|s: &mut App, tx| {
                s.undo_last(tx);
            })
            .child(text("Undo").font(style::BODY_BOLD).color(Color::WHITE))))
}

fn vline(p: Palette) -> El {
    el().w(1.0).h_full().fill(p.separator)
}

/// The search field's width: it gives up space down to its minimum before buttons go.
const SEARCH_W: f32 = 220.0;
const SEARCH_MIN_W: f32 = 120.0;

/// The toolbar's button groups, as button counts, in order of importance (which is also their
/// order on screen).
const TOOL_GROUPS: [usize; 6] = [2, 3, 3, 2, 2, 1];

/// How many of [`TOOL_GROUPS`] fit beside the search field in a toolbar `width` wide; the
/// rest are left out (their keyboard shortcuts still work). The first group always shows.
fn tool_groups_that_fit(width: f32) -> usize {
    // The row's padding, the search field at its narrowest with the space before it, and the
    // spacer's gaps.
    let mut used = list::WIDTH + 16.0 + SEARCH_MIN_W + 6.0 + 4.0;
    let mut n = 0;
    for (i, &k) in TOOL_GROUPS.iter().enumerate() {
        // Buttons 32 wide with 2 between them, and a 10 gap (plus its own 2s) before the group.
        let w = k as f32 * 32.0 + (k - 1) as f32 * 2.0 + if i > 0 { 14.0 } else { 0.0 };
        if i > 0 && used + w > width {
            break;
        }
        used += w;
        n += 1;
    }
    n
}

fn toolbar(s: &App, width: f32, p: Palette) -> El {
    let (name, total, unread) = s.view_summary();
    let summary = match (s.marked.len(), unread) {
        (n, _) if n > 0 => format!("{n} marked of {total}"),
        (_, 0) => format!("{total} messages"),
        (_, u) => format!("{total} messages, {u} unread"),
    };
    let has = s.has_targets();
    let one = s.selected.is_some() && s.marked.is_empty();
    let read_label = if s.selected_message().is_some_and(|m| m.seen) { "Mark as Unread" } else { "Mark as Read" };
    let title = col().w(list::WIDTH).px(16.0).gap(1.0)
        .child(text(name).font(TITLE).color(p.text).truncate())
        .child(text(summary).font(SMALL).color(p.secondary));
    let search = row().key("search").w(SEARCH_W).shrink().min_w(SEARCH_MIN_W).h(28.0).px(8.0).gap(6.0).center_y().radius(7.0).fill(p.field)
        .cursor(rune::Cursor::Text)
        .on_press(|s: &mut App| s.search.focused = true)
        .child(icon(&style::SEARCH, 14.0).color(p.secondary))
        .child(field::input(&s.search, BODY, p).key("search-field").single_line().grow(1.0)
            .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut Tx| {
                let took = field::platform_key(&mut s.search, e);
                if took && s.search.text.is_empty() {
                    s.run_search();
                }
                took
            })
            .placeholder("Search", p.secondary)
            .on_edit(|s: &mut App| &mut s.search)
            .on_input_change(|s: &mut App, _| if s.search.text.is_empty() { s.run_search() })
            .on_submit(|s: &mut App, tx: &mut Tx| {
                s.run_search();
                tx.blur();
            }));
    let groups = tool_groups_that_fit(width);
    let mut tools = row().grow(1.0).min_w(0.0).px(8.0).gap(2.0).center_y()
        .child(get_mail(s, p))
        .child(tool(&style::COMPOSE, "New Message (Ctrl+N)", true, p).on_press(|s: &mut App| s.compose_new()));
    if groups > 1 {
        tools = tools.child(gap())
            .child(tool(&style::ARCHIVE, "Archive (Ctrl+E)", has, p).on_press_tx(|s: &mut App, tx| s.move_targets("archive", tx)))
            .child(tool(&style::TRASH, "Delete", has, p).on_press_tx(|s: &mut App, tx| s.move_targets("trash", tx)))
            .child(tool(&style::JUNK, "Move to Junk", has, p).on_press_tx(|s: &mut App, tx| s.move_targets("junk", tx)));
    }
    if groups > 2 {
        tools = tools.child(gap())
            .child(tool(&style::REPLY, "Reply (Ctrl+R)", one, p).on_press(|s: &mut App| s.reply(false)))
            .child(tool(&style::REPLY_ALL, "Reply All (Ctrl+Shift+R)", one, p).on_press(|s: &mut App| s.reply(true)))
            .child(tool(&style::FORWARD, "Forward (Ctrl+Shift+F)", one, p).on_press(|s: &mut App| s.forward()));
    }
    if groups > 3 {
        tools = tools.child(gap())
            .child(tool(&style::FLAG, "Flag (Ctrl+Shift+L)", has, p).on_press(|s: &mut App| s.toggle_flag()))
            .child(tool(&style::ENVELOPE, read_label, has, p).on_press(|s: &mut App| s.toggle_read()));
    }
    if groups > 4 {
        tools = tools.child(gap())
            .child(tool(&style::UNSUBSCRIBE, "Unsubscribe (Ctrl+Shift+X)", one && s.unsubscribe_target().is_some(), p)
                .on_press_tx(|s: &mut App, tx| s.unsubscribe(tx)))
            .child(tool(&style::SPARKLES, if s.settings.vim { "Copy for AI (Ctrl+Shift+C or y)" } else { "Copy for AI (Ctrl+Shift+C)" }, has, p)
                .on_press_tx(|s: &mut App, tx| s.copy_for_ai(tx)));
    }
    tools = tools.child(el().grow(1.0));
    if groups > 5 {
        tools = tools.child(tool(&style::KEYBOARD, "Keyboard Shortcuts (Ctrl+/)", true, p)
            .on_press(|s: &mut App| s.sheet = Sheet::Shortcuts));
    }
    row().key("toolbar").w_full().h(52.0).center_y().fill(p.window)
        .child(title)
        .child(tools.child(el().w(6.0)).child(search))
}

/// Get New Mail: its arrows turn while a check is running.
fn get_mail(s: &App, p: Palette) -> El {
    let mut arrows = icon(&style::REFRESH, 16.0).color(p.secondary);
    if s.refreshing {
        arrows = arrows.looping(rune::LoopSpec::rotation(1.0));
    }
    let key = if s.settings.vim { "Ctrl+Shift+N or gr" } else { "Ctrl+Shift+N" };
    el().key("get-mail").size(32.0, 28.0).radius(6.0).center().hover(p.hover)
        .help(if s.refreshing { "Checking for new mail…".to_owned() } else { format!("Get New Mail ({key})") })
        .on_press(|s: &mut App| s.refresh())
        .child(arrows)
}

fn gap() -> El {
    el().w(10.0)
}

fn tool(i: &'static Icon, help: &str, enabled: bool, p: Palette) -> El {
    el().size(32.0, 28.0).radius(6.0).center().hover(p.hover).help(help)
        .opacity(if enabled { 1.0 } else { 0.35 })
        .child(icon(i, 17.0).color(p.secondary))
}
