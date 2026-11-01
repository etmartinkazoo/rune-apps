//! Adding an account: a Fastmail API token, a Gmail address and app password, or any IMAP
//! and SMTP servers. Full window on first run, a sheet after.

use rune::{Color, El, TextEdit, col, el, row, text};

use super::compose::button;
use super::style::{BODY, Palette, SMALL};
use crate::app::{AddForm, AddKind, App, Phase};

/// The first-run window.
pub fn view(s: &App, p: Palette) -> El {
    col().w_full().h_full().center().fill(p.sidebar).child(card(s, p, false).border(1.0, p.separator))
}

/// "Add Account…" from Settings, over the main window.
pub fn sheet(s: &App, p: Palette) -> El {
    let card = card(s, p, true)
        .border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.close_sheet());
    el().key("add-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

fn card(s: &App, p: Palette, adding: bool) -> El {
    let title = if adding { "Add Account" } else { "Rune Mail" };
    let mut card = col().key("signin").w(460.0).padding(28.0).gap(14.0).radius(14.0).fill(p.content)
        .child(text(title).font(rune::TextStyle::new(22.0, 700.0)).color(p.text));
    if s.phase == Phase::Connecting || (adding && s.is_connecting()) {
        return card.child(text("Connecting…").font(BODY).color(p.text));
    }
    card = card.child(kinds(s.add.kind, p));
    card = match s.add.kind {
        AddKind::Fastmail => card
            .child(text("In Fastmail, open Settings → Privacy & Security → Manage API tokens, and create a token \
                         with access to Email and Email submission.")
                .font(SMALL).color(p.secondary).wrap())
            .child(field(&s.token, "token", "API token (fmu1-…)", true, |s| &mut s.token, p)),
        AddKind::Gmail => card
            .child(text("Gmail needs an app password: turn on 2-Step Verification, then create one at \
                         myaccount.google.com/apppasswords.")
                .font(SMALL).color(p.secondary).wrap())
            .child(field(&s.add.email, "add-email", "you@gmail.com", false, |s| &mut s.add.email, p))
            .child(field(&s.add.name, "add-name", "Your name (optional)", false, |s| &mut s.add.name, p))
            .child(field(&s.add.password, "add-password", "App password", true, |s| &mut s.add.password, p)),
        AddKind::Other => card.children(other(&s.add, p)),
    };
    card = card.child(text("Kept in your system keyring (or, without one, in ~/.config/rune-mail, readable only by you).")
        .font(SMALL).color(p.tertiary).wrap());
    if let Some(e) = &s.signin_error {
        card = card.child(text(e.clone()).font(SMALL).color(rune::color::hex("#ff453a").unwrap()).wrap());
    }
    let connect = button("Connect", p.on_selection, p.accent, p).default_button().on_press(|s: &mut App| s.connect());
    if adding {
        card.child(row().w_full().gap(8.0).justify_end()
            .child(button("Cancel", p.text, p.field, p).on_press(|s: &mut App| s.close_sheet()))
            .child(connect))
    } else {
        card.child(connect.w_full())
    }
}

fn kinds(current: AddKind, p: Palette) -> El {
    let mut r = row().key("add-kind").w_full().h(28.0).padding(2.0).gap(2.0).radius(7.0).fill(p.field);
    for (label, kind) in [("Fastmail", AddKind::Fastmail), ("Gmail", AddKind::Gmail), ("Other (IMAP)", AddKind::Other)] {
        let on = kind == current;
        r = r.child(row().grow(1.0).h(24.0).center().radius(5.0)
            .fill(if on { p.content } else { Color::TRANSPARENT })
            .hover(p.hover)
            .on_press(move |s: &mut App| s.choose_add_kind(kind))
            .child(text(label).font(SMALL).color(if on { p.text } else { p.secondary })));
    }
    r
}

fn other(f: &AddForm, p: Palette) -> Vec<El> {
    let starttls = f.starttls;
    let save_sent = f.save_sent;
    vec![
        field(&f.email, "add-email", "Email address", false, |s| &mut s.add.email, p),
        field(&f.name, "add-name", "Your name (optional)", false, |s| &mut s.add.name, p),
        field(&f.password, "add-password", "Password", true, |s| &mut s.add.password, p),
        field(&f.login, "add-login", "User name (if not the address)", false, |s| &mut s.add.login, p),
        row().w_full().gap(8.0)
            .child(field(&f.imap_host, "add-imap", "IMAP server (imap.example.com)", false, |s| &mut s.add.imap_host, p).grow(1.0))
            .child(field(&f.imap_port, "add-imap-port", "993", false, |s| &mut s.add.imap_port, p).w(72.0)),
        row().w_full().gap(8.0)
            .child(field(&f.smtp_host, "add-smtp", "SMTP server (smtp.example.com)", false, |s| &mut s.add.smtp_host, p).grow(1.0))
            .child(field(&f.smtp_port, "add-smtp-port", "465", false, |s| &mut s.add.smtp_port, p).w(72.0)),
        row().w_full().gap(8.0).center_y()
            .child(check("SMTP uses STARTTLS (port 587)", starttls, p).on_press(|s: &mut App| {
                s.add.starttls = !s.add.starttls;
                s.add.smtp_port = TextEdit::new(if s.add.starttls { "587" } else { "465" });
            }))
            .child(check("Save sent mail to Sent", save_sent, p).on_press(|s: &mut App| s.add.save_sent = !s.add.save_sent)),
        text("IMAP connects with TLS on 993. Some providers save sent mail themselves; turn saving off for them.")
            .font(SMALL).color(p.tertiary).wrap(),
    ]
}

fn check(label: &str, on: bool, p: Palette) -> El {
    row().gap(6.0).center_y().radius(4.0).hover(p.hover)
        .child(el().size(14.0, 14.0).radius(3.0).fill(if on { p.accent } else { p.field }).border(1.0, p.separator))
        .child(text(label).font(SMALL).color(p.text))
}

fn field(edit: &TextEdit, key: &str, placeholder: &str, secure: bool, pick: fn(&mut App) -> &mut TextEdit, p: Palette) -> El {
    let mut input = super::field::input(edit, BODY, p).key(key).single_line().grow(1.0)
        .placeholder(placeholder, p.tertiary)
        .on_edit(move |s: &mut App| pick(s))
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(pick(s), e))
        .on_submit(|s: &mut App, _| s.connect());
    if secure {
        input = input.secure();
    }
    super::field::boxed(input, 30.0, 10.0, pick, p)
}
