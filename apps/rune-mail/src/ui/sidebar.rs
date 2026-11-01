//! The mailbox sidebar: Favorites, then each account's mailboxes, with today's events and
//! Settings at the bottom.

use rune::{Color, El, ScrollSpec, col, el, icon, row, text};

use super::style::{self, BODY, Palette, SECTION, SMALL};
use super::today;
use crate::app::{App, Item, Section, Sheet};
use crate::worker::View;

pub const WIDTH: f32 = 220.0;

pub fn view(s: &App, p: Palette) -> El {
    let mut list = col().key("sidebar-list").w_full().px(10.0).pb(12.0).gap(1.0);
    for section in s.sidebar() {
        if section.items.is_empty() {
            continue;
        }
        let collapsed = s.is_collapsed(&section.id);
        list = list.child(header(&section, collapsed, p));
        if !collapsed {
            for item in &section.items {
                list = list.child(mailbox_row(s, item, p));
            }
        }
    }
    let settings = row().key("open-settings").w_full().h(28.0).px(8.0).gap(8.0).center_y().radius(6.0)
        .hover(p.hover).help("Settings (Ctrl+,)")
        .on_press(|s: &mut App| s.sheet = Sheet::Settings)
        .child(icon(&style::GEAR, 16.0).color(p.secondary))
        .child(text("Settings").font(BODY).color(p.secondary));
    col().key("sidebar").w(WIDTH).h_full().fill(p.sidebar).pt(10.0)
        .child(list.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
        .child(today::view(s, p))
        .child(col().w_full().px(10.0).pb(10.0).child(settings))
}

/// A section's header: press it to fold or unfold the section. Folded, it shows the unread
/// count of what it hides (Favorites' inboxes are counted once, under All Inboxes or Inbox).
fn header(section: &Section, collapsed: bool, p: Palette) -> El {
    let id = section.id.clone();
    let mut r = row().key(format!("section-{}", section.id)).role("button").w_full().px(8.0).pt(14.0).pb(4.0).gap(4.0)
        .center_y().radius(5.0)
        .help(if collapsed { "Show" } else { "Hide" })
        .on_press_tx(move |s: &mut App, tx| s.toggle_section(&id, tx))
        .child(text(section.title.clone()).font(SECTION).color(p.secondary).truncate().min_w(0.0))
        .child(icon(if collapsed { &style::CHEVRON_RIGHT } else { &style::CHEVRON_DOWN }, 10.0).color(p.tertiary))
        .child(el().grow(1.0));
    let unread: u32 = section
        .items
        .iter()
        .filter(|i| (i.depth == 0 || section.id != "favorites") && !matches!(i.role.as_deref(), Some("trash" | "sent" | "drafts" | "junk")))
        .map(|i| i.unread)
        .sum();
    if collapsed && unread > 0 {
        r = r.child(text(unread.to_string()).font(SMALL).color(p.secondary).pr(8.0));
    }
    r
}

fn row_key(view: &View) -> String {
    match view {
        View::AllInboxes => "mb-all".to_owned(),
        View::Mailbox { account, id } => format!("mb-{account}-{id}"),
    }
}

fn mailbox_row(s: &App, item: &Item, p: Palette) -> El {
    let selected = s.view.as_ref() == Some(&item.view);
    let target = s.drop_over(&item.view);
    let view = item.view.clone();
    let (rect_key, rect_view) = (row_key(&item.view), item.view.clone());
    let counted = !matches!(item.role.as_deref(), Some("trash" | "sent" | "drafts"));
    let mut r = row().key(row_key(&item.view)).role("listitem").w_full().h(28.0)
        .pl(8.0 + item.depth as f32 * 14.0).pr(8.0).gap(8.0).center_y().radius(6.0)
        .fill(if target { p.accent.alpha(0.22) } else if selected { p.sidebar_selection } else { Color::TRANSPARENT })
        .border(1.0, if target { p.accent } else { Color::TRANSPARENT })
        .hover(p.hover)
        .on_press(move |s: &mut App| s.select_view(view.clone()))
        // Where it's drawn, so a dragged message can be dropped on it.
        .on_drawn_layout_tx(move |s: &mut App, r, tx| {
            s.rects.mailboxes.insert(rect_key.clone(), (rect_view.clone(), r));
            tx.live_only();
        })
        .child(icon(style::mailbox_icon(item.role.as_deref()), 16.0).color(p.accent))
        .child(text(item.name.clone()).font(BODY).color(p.text).grow(1.0).min_w(0.0).truncate());
    if item.unread > 0 && counted {
        r = r.child(text(item.unread.to_string()).font(SMALL).color(p.secondary));
    }
    r
}
