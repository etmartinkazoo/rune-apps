//! Choosing the phone: the ones already paired, pairing a new one (with the code both screens
//! show), and what to allow on the phone.

use rune::{Color, El, col, el, icon, row, text};

use super::style::{self, BIG, BODY, BODY_BOLD, HUGE, Palette, SMALL};
use super::button;
use crate::app::App;
use crate::bluez::Device;

pub fn view(s: &App, p: Palette) -> El {
    let phones: Vec<&Device> = s.devices.iter().filter(|d| d.phone).collect();
    let (paired, nearby): (Vec<&Device>, Vec<&Device>) = phones.into_iter().partition(|d| d.paired);
    let mut card = col().key("setup").w(460.0).padding(28.0).gap(14.0).radius(16.0).fill(p.card).border(1.0, p.separator)
        .child(row().gap(12.0).center_y()
            .child(el().size(48.0, 48.0).radius(12.0).center().fill(*style::GREEN).child(icon(&style::PHONE, 26.0).color(Color::WHITE)))
            .child(col()
                .child(text("Rune Phone").font(BIG).color(p.text))
                .child(text("Calls and texts from your phone, over Bluetooth").font(SMALL).color(p.secondary))));
    if let Some((name, code)) = &s.pair_code {
        let shown = format!("{:03} {:03}", code / 1000, code % 1000);
        return centred(card
            .child(text(format!("Does {} show this code?", if name.is_empty() { "the phone" } else { name })).font(BODY).color(p.text))
            .child(row().w_full().justify_center().py(8.0).child(text(shown).font(HUGE).color(p.text)))
            .child(text("On the phone, tap Pair, and allow access to contacts, call history and messages when it asks.").font(SMALL).color(p.secondary).wrap())
            .child(row().w_full().gap(8.0)
                .child(el().grow(1.0))
                .child(button("Cancel", p.text, p.field, p).key("reject").on_press(|s: &mut App| s.confirm(false)))
                .child(button("Pair", Color::WHITE, p.accent, p).key("confirm").on_press(|s: &mut App| s.confirm(true)))), p);
    }
    if !paired.is_empty() {
        card = card.child(text("YOUR PHONES").font(style::SECTION).color(p.tertiary));
        for d in paired {
            let d2 = d.clone();
            let current = s.link.phone.as_ref().is_some_and(|ph| ph.address == d.address);
            if s.removing.as_deref() == Some(d.address.as_str()) {
                let d3 = d.clone();
                let ask = if current { "Disconnect and unpair it?" } else { "Unpair it from this computer?" };
                card = card.child(device_row(d, Some(ask), p)
                    .child(button("Cancel", p.text, p.card, p).key(format!("keep-{}", d.address)).on_press(|s: &mut App| s.removing = None))
                    .child(button("Remove", Color::WHITE, *style::RED, p).key(format!("remove-{}", d.address)).on_press(move |s: &mut App| s.remove_phone(&d3))));
                continue;
            }
            let address = d.address.clone();
            card = card.child(device_row(d, None, p)
                .child(button("Remove", p.secondary, Color::TRANSPARENT, p).key(format!("ask-remove-{}", d.address))
                    .help("Unpair it from this computer").on_press(move |s: &mut App| s.removing = Some(address.clone())))
                .child(if current {
                    text("In use").font(SMALL).color(p.secondary)
                } else {
                    button("Use", Color::WHITE, p.accent, p).key(format!("use-{}", d.address)).on_press(move |s: &mut App| s.use_phone(&d2))
                }));
        }
    }
    card = card.child(row().w_full().gap(8.0).center_y().pt(6.0)
        .child(text("PAIR A NEW PHONE").font(style::SECTION).color(p.tertiary).grow(1.0))
        .child(if s.discovering { text("Looking…").font(SMALL).color(p.secondary) } else {
            button("Look for phones", p.text, p.field, p).key("discover").on_press(|s: &mut App| s.discover())
        }));
    if s.discovering || !nearby.is_empty() {
        if nearby.is_empty() {
            card = card.child(text("On the phone, open Settings › Connected devices › Pair new device, and keep that screen open.")
                .font(SMALL).color(p.secondary).wrap());
        }
        for d in nearby {
            let d2 = d.clone();
            let busy = s.pairing.as_deref() == Some(d.path.as_str());
            card = card.child(device_row(d, None, p).child(if busy {
                text("Pairing…").font(SMALL).color(p.secondary)
            } else {
                button("Pair", p.text, p.field, p).key(format!("pair-{}", d.address)).on_press(move |s: &mut App| s.pair(&d2))
            }));
        }
    }
    card = card.child(text("Rune Phone uses the phone's own Bluetooth services: no app to install on it. Texts are SMS and MMS; \
        RCS chats stay in Google Messages on the phone.").font(SMALL).color(p.tertiary).wrap());
    if s.settings.phone.is_some() {
        card = card.child(row().w_full().child(el().grow(1.0)).child(button("Done", p.text, p.field, p).key("done").on_press(|s: &mut App| s.setup_open = false)));
    }
    centred(card, p)
}

/// A phone: its name, and how it is (or `note` in its place, a question about it).
fn device_row(d: &Device, note: Option<&str>, p: Palette) -> El {
    let state = note.unwrap_or(if d.connected { "Connected" } else if d.paired { "Paired" } else { "Nearby" });
    row().key(format!("device-{}", d.address)).w_full().px(12.0).py(10.0).gap(10.0).center_y().radius(10.0).fill(p.field)
        .child(icon(&style::PHONE, 16.0).color(p.secondary))
        .child(col().grow(1.0).min_w(0.0)
            .child(text(if d.name.is_empty() { d.address.clone() } else { d.name.clone() }).font(BODY_BOLD).color(p.text).truncate())
            .child(text(state.to_owned()).font(SMALL).color(p.secondary).truncate()))
}

fn centred(c: El, p: Palette) -> El {
    col().w_full().h_full().fill(p.window).center().child(c)
}
