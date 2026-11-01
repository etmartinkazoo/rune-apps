//! Signing in to a Vaultwarden server, its two-step code, and unlocking the vault.

use rune::{Color, El, TextEdit, Tx, col, el, icon, row, text};

use super::field::{boxed, input, platform_key};
use super::style::{self, BIG, BODY, BODY_BOLD, Palette, SMALL};
use crate::app::App;

fn card(p: Palette) -> El {
    col().w(380.0).padding(28.0).gap(12.0).radius(14.0).fill(p.card).border(1.0, p.separator).items_center()
}

fn badge(p: Palette) -> El {
    el().size(64.0, 64.0).radius(16.0).center().fill(*style::GREY).child(icon(&style::KEY, 34.0).color(Color::WHITE)).mb(4.0)
        .shadow(2.0, 8.0, Color::BLACK.alpha(if p.dark { 0.4 } else { 0.12 }))
}

/// A labelled field of the form; Return moves on with `next`.
fn form_field(
    edit: &TextEdit,
    key: &str,
    hint: &str,
    pick: fn(&mut App) -> &mut TextEdit,
    next: fn(&mut App, &mut Tx),
    p: Palette,
) -> El {
    let i = input(edit, BODY, p).key(key.to_owned()).single_line().grow(1.0).min_w(0.0).placeholder(hint, p.tertiary)
        .on_edit(pick)
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, _: &mut Tx| platform_key(pick(s), e))
        .on_submit(next);
    boxed(i, 30.0, 10.0, pick, p)
}

fn primary(label: &str, busy: bool, p: Palette) -> El {
    row().key("go").w_full().h(30.0).radius(7.0).center().fill(if busy { p.accent.alpha(0.6) } else { p.accent })
        .hover(p.accent.mix(Color::BLACK, 0.12))
        .child(text(if busy { "Working…" } else { label }).font(BODY_BOLD).color(Color::WHITE))
}

pub fn sign_in(s: &App, p: Palette) -> El {
    let f = &s.forms;
    let mut c = card(p)
        .child(badge(p))
        .child(text(if s.adding { "Add an Account" } else { "Rune Passwords" }).font(BIG).color(p.text))
        .child(text("Sign in to your Vaultwarden (or Bitwarden) server. Your vault is decrypted only on this computer.")
            .font(SMALL).color(p.secondary).wrap().align(rune::Align::Center));
    if let Some(providers) = &f.needs_code {
        let how = match providers.first() {
            Some(1) => "Enter the code the server emailed you.",
            _ => "Enter the code from your authenticator app.",
        };
        c = c.child(text(how).font(BODY).color(p.text).wrap())
            .child(form_field(&f.code, "code", "Verification code", |s| &mut s.forms.code, |s, _| s.sign_in(), p));
    } else {
        c = c
            .child(form_field(&f.server, "server", "Server, as https://vault.example.com", |s| &mut s.forms.server, |s, _| s.forms.email.focused = true, p))
            .child(form_field(&f.email, "email", "Email address", |s| &mut s.forms.email, |s, _| s.forms.password.focused = true, p))
            .child(form_field(&f.password, "password", "Master password", |s| &mut s.forms.password, |s, _| s.sign_in(), p));
    }
    if let Some(e) = &s.error {
        c = c.child(text(e.clone()).font(SMALL).color(p.danger).wrap());
    }
    let go = primary(if f.needs_code.is_some() { "Continue" } else { "Sign In" }, s.busy, p);
    c = c.child(if s.busy { go } else { go.on_press(|s: &mut App| s.sign_in()) });
    if s.adding && s.account_id.is_some() {
        c = c.child(text("Cancel").font(SMALL).color(p.accent).key("cancel-add").on_press(|s: &mut App| s.cancel_add()));
    }
    centred(c, p)
}

pub fn unlock(s: &App, p: Palette) -> El {
    let mut c = card(p)
        .child(badge(p))
        .child(text("Rune Passwords is locked").font(BIG).color(p.text))
        .child(text(format!("Enter the master password for {}.", s.email)).font(SMALL).color(p.secondary).wrap().align(rune::Align::Center))
        .child(form_field(&s.forms.unlock, "unlock", "Master password", |s| &mut s.forms.unlock, |s, _| s.unlock(), p));
    if let Some(e) = &s.error {
        c = c.child(text(e.clone()).font(SMALL).color(p.danger).wrap());
    }
    let go = primary("Unlock", s.busy, p);
    c = c.child(if s.busy { go } else { go.on_press(|s: &mut App| s.unlock()) })
        .child(row().gap(16.0)
            .child(text("Add Account").font(SMALL).color(p.accent).key("add-account").on_press(|s: &mut App| s.add_account()))
            .child(text("Sign Out").font(SMALL).color(p.accent).key("sign-out").on_press(|s: &mut App| s.sign_out())));
    let others: Vec<_> = s.accounts.iter().filter(|a| s.account_id.as_deref() != Some(a.id.as_str())).collect();
    if !others.is_empty() {
        c = c.child(el().w_full().h(1.0).fill(p.separator).mt(4.0))
            .child(text("Other accounts").font(SMALL).color(p.secondary).w_full());
        for a in others {
            let id = a.id.clone();
            c = c.child(super::sidebar::account_row(a, false, p).on_press(move |s: &mut App| s.switch_account(&id)));
        }
    }
    centred(c, p)
}

fn centred(c: El, p: Palette) -> El {
    col().w_full().h_full().fill(p.window).center().child(c).child(row().h(40.0))
}
