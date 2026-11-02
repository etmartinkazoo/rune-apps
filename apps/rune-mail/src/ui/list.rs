//! The message list: sender, date, subject and a preview, with unread dots, flags and marks.

use rune::{Color, El, GrowthSpec, ScrollSpec, col, el, icon, row, text};

use super::dates;
use super::style::{self, BODY, BODY_BOLD, Palette, SMALL};
use crate::app::{App, Pane};

pub const WIDTH: f32 = 340.0;

pub fn view(s: &App, p: Palette) -> El {
    let pane = col().key("list-pane").w(WIDTH).h_full().fill(p.content);
    if s.messages.is_empty() {
        let note = if s.list_loading { "Loading…" } else { "No Messages" };
        return pane.center().child(text(note).font(BODY).color(p.secondary));
    }
    // The first row sits as far from the top as rows do from the side. The space is the list's,
    // not the first row's: a row that became first (the one above archived) would otherwise
    // grow by it at once, jumping every row below down before they slide up.
    let rows = col().key("rows").w_full().pt(8.0).growth(GrowthSpec::default())
        .virtual_rows(
            s.messages.len(),
            40.0 + SMALL.line_px() * s.settings.preview_lines as f32,
            |s: &App, i| format!("msg-{}", s.messages.get(i).map_or("", |m| m.uid.as_str())).into(),
            move |s: &App, i| message_row(s, i, p),
        )
        .virtual_rows_version(s.list_version);
    pane.child(col().key("list-scroll").w_full().grow(1.0).min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false }).child(rows))
}

fn message_row(s: &App, i: usize, p: Palette) -> El {
    let Some(m) = s.messages.get(i) else { return el() };
    // While the keyboard is in the conversation the selection goes grey, as an unfocused
    // list's does on macOS.
    let current = s.selected.as_ref() == Some(&m.uid);
    let selected = current && s.pane == Pane::List;
    let resting = current && !selected;
    let marked = s.marked.contains(&m.uid);
    let (ink, muted) = if selected { (p.on_selection, p.on_selection.alpha(0.8)) } else { (p.text, p.secondary) };
    let sender = m.from.first().map_or("(no sender)".to_owned(), |a| a.display().to_owned());
    let subject = if m.subject.is_empty() { "(no subject)".to_owned() } else { m.subject.clone() };
    // The row stands for its conversation: unread, flagged or with attachments if any is.
    let thread = s.thread(m);
    let unread = thread.iter().any(|t| !t.seen);
    let flagged = thread.iter().any(|t| t.flagged);
    let attachment = thread.iter().any(|t| t.has_attachment);

    let mut gutter = col().w(22.0).pt(3.0).gap(6.0).items_center();
    if marked {
        gutter = gutter.child(el().size(14.0, 14.0).radius(7.0).fill(if selected { ink } else { p.accent }).center()
            .child(icon(&style::CHECK, 10.0).color(if selected { p.selection } else { p.on_selection })));
    } else if unread {
        gutter = gutter.child(el().size(9.0, 9.0).radius(4.5).fill(if selected { ink } else { p.accent }));
    }
    if flagged {
        gutter = gutter.child(icon(&style::FLAG, 11.0).color(if selected { ink } else { p.flag }));
    }

    let mut top = row().w_full().gap(6.0).center_y()
        .child(text(sender).font(BODY_BOLD).color(ink).grow(1.0).min_w(0.0).truncate());
    if attachment {
        top = top.child(icon(&style::CLIP, 12.0).color(muted));
    }
    top = top.child(text(dates::short(&m.received_at)).font(SMALL).color(muted));

    let mut content = col().grow(1.0).min_w(0.0).gap(2.0)
        .child(top)
        .child({
            let mut line = row().w_full().gap(6.0).center_y()
                .child(text(subject).font(BODY).color(ink).grow(1.0).min_w(0.0).truncate());
            if thread.len() > 1 {
                line = line.child(row().h(16.0).px(6.0).radius(8.0).center()
                    .fill(if selected { p.on_selection.alpha(0.25) } else { p.field })
                    .child(text(thread.len().to_string()).font(style::COUNT).color(muted)));
            }
            line
        });
    let lines = s.settings.preview_lines;
    // The preview always takes its full height, empty or short, so every row is the same size.
    if lines > 0 {
        content = content.child(col().w_full().h(SMALL.line_px() * lines as f32)
            .child(text(m.preview.clone()).font(SMALL).color(muted).wrap_anywhere().max_lines(lines)));
    }

    let fill = if selected {
        p.selection
    } else if resting {
        p.sidebar_selection
    } else if marked {
        p.accent.alpha(0.14)
    } else {
        Color::TRANSPARENT
    };
    let offset = s.swipe_offset(&m.uid);
    let dragged = s.dragging.as_ref().is_some_and(|d| d.rows.contains(&m.uid));
    // Handlers find their row by uid when they run: by then a refresh may have moved it.
    let (wheel_uid, rect_uid, press_uid, drag_uid) = (m.uid.clone(), m.uid.clone(), m.uid.clone(), m.uid.clone());
    let at = |s: &App, uid: &str| s.messages.iter().position(|m| m.uid == uid);
    // Slid by a swipe, the card covers its buttons with a solid fill, square where it meets them
    // (the row clips the outer corners round).
    let card = row().key("card").w_full().py(9.0).pr(12.0).items_start().radius(if offset != 0.0 { 0.0 } else { 8.0 })
        .fill(if offset != 0.0 && fill == Color::TRANSPARENT { p.content } else { fill })
        .hover(if selected { Color::TRANSPARENT } else { p.hover })
        .offset(offset, 0.0)
        .opacity(if dragged { 0.45 } else { 1.0 })
        .on_press_tx(move |s: &mut App, tx| {
            if let Some(i) = at(s, &press_uid) {
                s.click_row(i, tx);
            }
        })
        .on_wheel(move |s: &mut App, e, tx| s.swipe_wheel(&wheel_uid, e, tx))
        .on_drag(move |s: &mut App, d, tx| {
            if let Some(i) = at(s, &drag_uid) {
                s.row_drag(i, d, tx);
            }
        })
        .on_drawn_layout_tx(move |s: &mut App, r, tx| {
            s.rects.rows.insert(rect_uid.clone(), r);
            tx.live_only();
        })
        .child(gutter)
        .child(content);
    let mut stack = el().w_full().radius(8.0).clip_rounded(8.0);
    if offset != 0.0 {
        stack = stack.child(swipe_buttons(s, m, offset, p));
    }
    let divider = row().w_full().h(1.0).pl(30.0).pr(8.0)
        .child(el().grow(1.0).h(1.0).fill(if current || marked { Color::TRANSPARENT } else { p.separator }));
    col().key(format!("msg-{}", m.uid)).w_full().px(8.0).child(stack.child(card)).child(divider)
}

/// The buttons a swipe uncovers: Flag, Archive and Delete on the right (swiping left), Read
/// or Unread on the left (swiping right). Swiped far enough to act, the action's colour fills
/// the row.
fn swipe_buttons(s: &App, m: &crate::jmap::Summary, offset: f32, p: Palette) -> El {
    use crate::gestures::{BUTTON, OPEN_LEFT, OPEN_RIGHT};
    let unread = s.thread(m).iter().any(|t| !t.seen);
    let button = |action: &'static str, label: &'static str, icon_: &'static rune::Icon, color: Color, w: f32| {
        let uid = m.uid.clone();
        col().key(format!("swipe-{action}")).w(w).h_full().center().gap(3.0).fill(color)
            .on_press_tx(move |s: &mut App, tx| s.swipe_action(&uid, action, tx))
            .child(icon(icon_, 16.0).color(Color::WHITE))
            .child(text(label).font(style::COUNT).color(Color::WHITE))
    };
    let archive = rune::color::hex("#8e5bd6").unwrap();
    let delete = rune::color::hex("#ff3b30").unwrap();
    if offset < 0.0 {
        let room = -offset;
        let (full_action, full_label, full_icon, full_color) = match s.settings.swipe_left {
            crate::settings::SwipeLeft::Archive => ("archive", "Archive", &style::ARCHIVE, archive),
            crate::settings::SwipeLeft::Delete => ("trash", "Delete", &style::TRASH, delete),
        };
        let r = row().pin(None, Some(0.0), Some(0.0), Some(0.0)).w(room.max(OPEN_LEFT)).justify_end();
        if room >= 220.0 {
            r.child(button(full_action, full_label, full_icon, full_color, room))
        } else {
            let w = (room / 3.0).min(BUTTON);
            r.child(button("flag", "Flag", &style::FLAG, p.flag, w))
                .child(button("archive", "Archive", &style::ARCHIVE, archive, w))
                .child(button("trash", "Delete", &style::TRASH, delete, w))
        }
    } else {
        let label = if unread { "Read" } else { "Unread" };
        row().pin(Some(0.0), Some(0.0), None, Some(0.0)).w(offset.max(OPEN_RIGHT))
            .child(button("read", label, &style::ENVELOPE, p.accent, offset.max(OPEN_RIGHT)))
    }
}
