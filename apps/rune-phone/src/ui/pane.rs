//! The right side: a conversation and its reply field, a new message, the keypad, or a contact.

use rune::{Color, Drag, El, Phase, ScrollSpec, TextEdit, Tx, col, el, icon, row, text};

use super::field::{input, platform_key};
use super::style::{self, BIG, BODY, BUBBLE, HUGE, KEY, NAME, Palette, SMALL};
use super::sidebar::small_tool;
use super::{avatar, button, round, swipeable, tool};
use crate::actions::{Focus, Target, handle};
use crate::app::{App, Pane, when};
use crate::number;

pub fn view(s: &App, p: Palette) -> El {
    let body = match &s.pane {
        Pane::Thread(key) => thread(s, key, p),
        Pane::Compose => compose(s, p),
        Pane::Keypad => keypad(s, p),
        Pane::Contact(i) => match s.contacts.get(*i) {
            Some(_) => contact(s, *i, p),
            None => nothing(p),
        },
        Pane::Nothing => nothing(p),
    };
    let mut c = col().key("pane").grow(1.0).min_w(0.0).h_full().fill(p.content);
    if s.current().is_some() {
        c = c.child(super::call::bar(s, p));
    }
    c.child(body)
}

fn nothing(p: Palette) -> El {
    col().w_full().grow(1.0).center().gap(8.0)
        .child(icon(&style::MESSAGE, 40.0).color(p.tertiary))
        .child(text("Choose a conversation, or start a new one").font(BODY).color(p.secondary))
        .child(text("Press ? for the keyboard shortcuts").font(SMALL).color(p.tertiary))
}

fn header(title: El, sub: String, number: String, p: Palette) -> El {
    let call = number.clone();
    row().w_full().px(20.0).py(12.0).gap(12.0).center_y()
        .child(col().grow(1.0).min_w(0.0).child(title).child(text(sub).font(SMALL).color(p.secondary)))
        .child(tool(&style::PHONE, "Call", p).key("call").on_press_tx(move |s: &mut App, tx: &mut Tx| s.call(&call, tx)))
}

fn thread(s: &App, key: &str, p: Palette) -> El {
    let Some(t) = s.thread(key) else { return compose(s, p) };
    let name = if t.name.is_empty() { s.who(&t.number) } else { t.name.clone() };
    let mut bubbles = col().key(format!("bubbles-{key}")).w_full().px(20.0).py(12.0).gap(4.0);
    let mut last_day = String::new();
    for m in &t.messages {
        let day = m.at.get(..8).unwrap_or("").to_owned();
        if day != last_day {
            bubbles = bubbles.child(row().w_full().justify_center().pt(10.0).pb(4.0).child(text(when(&m.at)).font(SMALL).color(p.tertiary)));
            last_day = day;
        }
        bubbles = bubbles.child(bubble(s, m, p));
    }
    col().w_full().grow(1.0).min_h(0.0)
        .child(header(text(name).font(NAME).color(p.text).truncate(), number::pretty(&t.number), t.number.clone(), p))
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(col().key("conversation").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: true }).child(bubbles))
        .child(if s.picked.messages.is_empty() { el() } else { picked_bar(s, p) })
        .child(reply(s, p))
}

/// What can be done with the messages picked.
fn picked_bar(s: &App, p: Palette) -> El {
    let n = s.picked.messages.len();
    let unread = s.messages.iter().any(|m| s.picked.messages.contains(&m.id) && m.incoming && !m.read);
    row().key("picked-messages").w_full().px(20.0).py(6.0).gap(4.0).center_y().fill(p.card)
        .child(text(format!("{n} selected")).font(BODY).color(p.text).grow(1.0))
        .child(small_tool(&style::MAIL, if unread { "Mark read (u)" } else { "Mark unread (u)" }, p.secondary, p).key("picked-msg-read")
            .on_press_tx(|s: &mut App, tx: &mut Tx| s.toggle_read(&s.picked_messages(), tx)))
        .child(small_tool(&style::TRASH, "Delete (dd)", *style::RED, p).key("picked-msg-delete")
            .on_press_tx(|s: &mut App, tx: &mut Tx| s.ask_delete(&s.picked_messages(), tx)))
        .child(small_tool(&style::CLOSE, "Let go of them (Escape)", p.secondary, p).key("picked-msg-clear")
            .on_press(|s: &mut App| s.picked.messages.clear()))
}

fn bubble(s: &App, m: &crate::worker::Message, p: Palette) -> El {
    let (fill, ink) = if m.incoming { (style::received(&p), p.text) } else { (p.accent, Color::WHITE) };
    let picked = s.picked.messages.contains(&m.id);
    let cursor = s.focus == Focus::Thread && s.msg_cursor.as_deref() == Some(m.id.as_str());
    let mut b = col().max_w(460.0).px(12.0).py(7.0).radius(16.0).fill(fill).gap(4.0);
    if picked {
        b = b.outline(2.5, 2.0, p.accent);
    } else if cursor {
        b = b.outline(1.5, 2.0, p.secondary);
    }
    if m.incoming && !m.read {
        b = b.border(1.5, p.accent);
    }
    if m.mms {
        b = b.child(row().gap(6.0).center_y().child(icon(&style::PHOTO, 14.0).color(ink)).child(text("Photo (on your phone)").font(SMALL).color(ink)));
    }
    if !m.text.is_empty() {
        b = b.child(text(m.text.clone()).font(BUBBLE).color(ink).wrap().selectable(true));
    }
    let target = Target::Message(m.id.clone());
    let (id, drag_target, wheel_target) = (m.id.clone(), target.clone(), target.clone());
    let mut r = row().key(format!("msg-{}", handle(&m.id))).w_full().py(1.0).items_center();
    if s.swiped_by(&target) != 0.0 {
        r = r.fill(p.content);
    }
    let check = el().size(18.0, 18.0).radius(9.0).center().fill(p.accent).child(icon(&style::CHECK, 12.0).color(Color::WHITE));
    r = r.on_press_tx(move |s: &mut App, tx: &mut Tx| s.click_message(&id, tx))
        .on_drag(move |s: &mut App, d: &mut Drag, tx: &mut Tx| match d.phase {
            Phase::Down => s.press_down(),
            Phase::Move => {
                tx.animated();
                s.drag_swipe(drag_target.clone(), d.dx, d.dy);
            }
            Phase::Up => s.end_swipe(d.cancelled(), tx),
        })
        .on_wheel(move |s: &mut App, e: rune::WheelEvent, tx: &mut Tx| s.wheel_swipe(wheel_target.clone(), &e, tx));
    // The check sits at the left, beside the bubble it's for when that's on the left too.
    let r = r.gap(8.0).child(if picked { check } else { el() });
    let r = if m.incoming { r.child(b).child(el().grow(1.0)) } else { r.child(el().grow(1.0)).child(b) };
    swipeable(s, target, r, 16.0, 0.0, p)
}

/// The field to write in and its send button; Return sends, Shift+Return breaks the line.
fn reply(s: &App, p: Palette) -> El {
    let can = !s.draft.text.trim().is_empty() && !s.sending && s.link.messages;
    let field = input(&s.draft, BUBBLE, p).key("draft").wrap().grow(1.0).min_w(0.0)
        .placeholder(if s.link.messages { "Text Message" } else { "Messages aren't connected" }, p.tertiary)
        .on_edit(|s: &mut App| &mut s.draft)
        .on_input_key(|s: &mut App, e: &rune::KeyEvent, tx: &mut Tx| {
            if e.key == rune::Key::Enter && !e.mods.shift {
                s.send_draft(tx);
                return true;
            }
            platform_key(&mut s.draft, e)
        });
    let send = round(&style::SEND, if can { p.accent } else { p.field }, if can { Color::WHITE } else { p.tertiary }, 28.0).key("send");
    row().w_full().px(16.0).py(10.0).gap(8.0).items_end()
        .child(row().grow(1.0).min_w(0.0).min_h(32.0).px(12.0).py(6.0).radius(16.0).border(1.0, p.separator).center_y()
            .cursor(rune::Cursor::Text).on_press(|s: &mut App| s.draft.focused = true).child(field))
        .child(if can { send.on_press_tx(|s: &mut App, tx: &mut Tx| s.send_draft(tx)) } else { send })
}

fn compose(s: &App, p: Palette) -> El {
    let to = input(&s.to, BODY, p).key("to").single_line().grow(1.0).min_w(0.0).placeholder("Name or number", p.tertiary)
        .on_edit(|s: &mut App| &mut s.to)
        .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut Tx| {
            if e.key == rune::Key::Enter {
                s.draft.focused = true;
                return true;
            }
            platform_key(&mut s.to, e)
        });
    let mut c = col().w_full().grow(1.0).min_h(0.0)
        .child(row().w_full().px(20.0).py(12.0).gap(8.0).center_y().child(text("To:").font(BODY).color(p.secondary)).child(to))
        .child(el().w_full().h(1.0).fill(p.separator));
    let q = s.to.text.trim().to_lowercase();
    if !q.is_empty() {
        let mut list = col().w_full().px(12.0).py(6.0);
        for (i, c2) in s.contacts.iter().filter(|c| c.name.to_lowercase().contains(&q)).take(6).enumerate() {
            for (label, n) in c2.numbers.iter().take(2) {
                let n2 = n.clone();
                list = list.child(row().key(format!("suggest-{i}-{n}")).w_full().px(8.0).py(6.0).gap(10.0).center_y().radius(6.0).hover(p.hover)
                    .on_press(move |s: &mut App| {
                        s.to = TextEdit::new(&n2);
                        s.draft.focused = true;
                    })
                    .child(avatar(&c2.name, 28.0))
                    .child(text(c2.name.clone()).font(BODY).color(p.text).grow(1.0).min_w(0.0).truncate())
                    .child(text(format!("{label} {}", number::pretty(n))).font(SMALL).color(p.secondary)));
            }
        }
        c = c.child(list);
    }
    c.child(el().grow(1.0)).child(reply(s, p))
}

const KEYS: [[(&str, &str); 3]; 4] = [
    [("1", ""), ("2", "ABC"), ("3", "DEF")],
    [("4", "GHI"), ("5", "JKL"), ("6", "MNO")],
    [("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ")],
    [("*", ""), ("0", "+"), ("#", "")],
];

/// A keypad: to dial, or (`small`) to send tones during a call.
pub fn pad(p: Palette, size: f32) -> El {
    let mut grid = col().gap(size * 0.2).items_center();
    for line in KEYS {
        let mut r = row().gap(size * 0.3);
        for (digit, letters) in line {
            let k = digit.to_owned();
            let mut key = col().key(format!("key-{digit}")).size(size, size).radius(size / 2.0).center().fill(p.field).hover(p.hover)
                .on_press(move |s: &mut App| s.press_key(&k))
                .child(text(digit.to_owned()).font(rune::TextStyle::new(size * 0.4, 400.0)).color(p.text));
            if !letters.is_empty() && size > 50.0 {
                key = key.child(text(letters.to_owned()).font(rune::TextStyle::new(9.0, 600.0)).color(p.secondary));
            }
            r = r.child(key);
        }
        grid = grid.child(r);
    }
    grid
}

fn keypad(s: &App, p: Palette) -> El {
    let shown = if s.dial.text.is_empty() { " ".to_owned() } else { number::pretty(&s.dial.text) };
    let who = if s.dial.text.len() >= 3 { s.who(&s.dial.text) } else { String::new() };
    let can = number::dialable(&s.dial.text).is_some() && s.link.calls;
    let call = round(&style::PHONE, if can { *style::GREEN } else { p.field }, if can { Color::WHITE } else { p.tertiary }, 64.0).key("dial");
    col().w_full().grow(1.0).center().gap(18.0)
        .child(text(shown).font(HUGE).color(p.text))
        .child(text(if who == number::pretty(&s.dial.text) { String::new() } else { who }).font(BODY).color(p.secondary))
        .child(pad(p, 68.0))
        .child(row().gap(24.0).center_y()
            .child(el().w(44.0))
            .child(if can {
                call.on_press_tx(|s: &mut App, tx: &mut Tx| {
                    let n = s.dial.text.clone();
                    s.call(&n, tx);
                })
            } else {
                call
            })
            .child(el().size(44.0, 44.0).radius(22.0).center().hover(p.hover).key("backspace")
                .on_press(|s: &mut App| {
                    let mut t = s.dial.text.clone();
                    t.pop();
                    s.dial = TextEdit::new(&t);
                })
                .child(text("⌫").font(KEY).color(p.secondary))))
        .child(if s.link.calls { el() } else { text("Calls need the phone connected over Bluetooth").font(SMALL).color(p.secondary) })
}

fn contact(s: &App, i: usize, p: Palette) -> El {
    let c = &s.contacts[i];
    let mut card = col().w_full().max_w(420.0).gap(10.0).items_center().pt(40.0)
        .child(avatar(&c.name, 88.0))
        .child(text(c.name.clone()).font(BIG).color(p.text));
    for (label, n) in &c.numbers {
        let (m, k) = (n.clone(), n.clone());
        card = card.child(row().key(format!("number-{n}")).w_full().px(16.0).py(10.0).gap(10.0).center_y().radius(10.0).fill(p.card)
            .child(col().grow(1.0).min_w(0.0)
                .child(text(label.clone()).font(SMALL).color(p.secondary))
                .child(text(number::pretty(n)).font(BODY).color(p.text)))
            .child(button("Message", p.text, p.field, p).on_press(move |s: &mut App| s.message(&m)))
            .child(round(&style::PHONE, *style::GREEN, Color::WHITE, 30.0).on_press_tx(move |s: &mut App, tx: &mut Tx| s.call(&k, tx))));
    }
    row().w_full().grow(1.0).justify_center().child(card)
}
