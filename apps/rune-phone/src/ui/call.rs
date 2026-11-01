//! A call: the card that rings over everything, and the bar that stays while one goes on.

use rune::{Color, El, col, el, row, text};

use super::style::{self, BIG, BODY, NAME, Palette, SMALL};
use super::{avatar, round};
use crate::app::App;
use crate::calls::{LiveCall, State};

fn who(s: &App, c: &LiveCall) -> String {
    if c.number == "withheld" || c.number.is_empty() {
        "No Caller ID".into()
    } else if !c.name.is_empty() {
        c.name.clone()
    } else {
        s.who(&c.number)
    }
}

/// A call ringing: who it is, Decline and Accept.
pub fn incoming(s: &App, c: &LiveCall, p: Palette) -> El {
    let name = who(s, c);
    let card = col().key("incoming").w(340.0).padding(24.0).gap(10.0).items_center().radius(18.0).fill(p.card)
        .border(1.0, p.separator).shadow(18.0, 48.0, Color::BLACK.alpha(0.35))
        .child(avatar(&name, 64.0))
        .child(text(name).font(BIG).color(p.text))
        .child(text(if c.state == State::Waiting { "Call waiting" } else { "Incoming call" }).font(BODY).color(p.secondary))
        .child(row().gap(40.0).pt(10.0)
            .child(col().gap(6.0).items_center()
                .child(round(&style::HANG_UP, *style::RED, Color::WHITE, 56.0).key("decline").on_press(|s: &mut App| s.send_cmd(crate::worker::Cmd::HangUp)))
                .child(text("Decline").font(SMALL).color(p.secondary)))
            .child(col().gap(6.0).items_center()
                .child(round(&style::PHONE, *style::GREEN, Color::WHITE, 56.0).key("accept").on_press(|s: &mut App| s.send_cmd(crate::worker::Cmd::Answer)))
                .child(text("Accept").font(SMALL).color(p.secondary))));
    rune::row().pin(Some(0.0), Some(24.0), Some(0.0), None).justify_center().child(card)
}

fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The call going on: who, how long, mute, the keypad for menus, and hang up.
pub fn bar(s: &App, p: Palette) -> El {
    let Some(c) = s.current() else { return el() };
    let state = match c.state {
        State::Dialing => "Calling…".to_owned(),
        State::Alerting => "Ringing…".to_owned(),
        State::Held => "On hold".to_owned(),
        _ => s.call_started.map_or(String::new(), |t| clock((s.clock - t).max(0.0) as u64)),
    };
    // A control that's on is filled with the text colour, its glyph the window's: dark on light
    // in dark mode, light on dark in light mode.
    let toggle = |on: bool| if on { (p.text, p.window) } else { (p.field, p.text) };
    let (mic_fill, mic_ink) = toggle(s.muted);
    let (pad_fill, pad_ink) = toggle(s.call_keypad);
    let mut c2 = col().key("call-bar").w_full().fill(p.card).px(20.0).py(10.0).gap(10.0)
        .child(row().w_full().gap(12.0).center_y()
            .child(avatar(&who(s, c), 36.0))
            .child(col().grow(1.0).min_w(0.0)
                .child(text(who(s, c)).font(NAME).color(p.text).truncate())
                .child(text(state).font(SMALL).color(p.secondary)))
            .child(round(if s.muted { &style::MIC_OFF } else { &style::MIC }, mic_fill, mic_ink, 36.0).key("mute").help(if s.muted { "Unmute" } else { "Mute" }).on_press(|s: &mut App| s.toggle_mute()))
            .child(round(&style::KEYPAD, pad_fill, pad_ink, 36.0)
                .key("call-keypad").help("Keypad").on_press(|s: &mut App| s.call_keypad = !s.call_keypad))
            .child(round(&style::HANG_UP, *style::RED, Color::WHITE, 36.0).key("end").help("End").on_press(|s: &mut App| s.send_cmd(crate::worker::Cmd::HangUp))));
    if s.call_keypad {
        c2 = c2.child(row().w_full().justify_center().pb(6.0).child(super::pane::pad(p, 44.0)));
    }
    c2.child(el().w_full().h(1.0).fill(p.separator))
}
