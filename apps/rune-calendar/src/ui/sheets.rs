//! Settings (a sidebar of panes, as Rune Mail's) and Add Account.

use rune::{Color, El, ScrollSpec, TextEdit, col, el, icon, row, text};

use super::style::{self, BODY, BODY_BOLD, Palette, SMALL};
use super::{button, segmented};
use crate::app::{AccountState, AddForm, App, Pane, Status};
use crate::caldav::Provider;
use crate::settings::{Appearance, View, WeekStart};

const PANES: [(Pane, &str); 2] = [(Pane::General, "General"), (Pane::Accounts, "Accounts")];

fn scrim(key: &str, card: El, p: Palette) -> El {
    el().key(format!("{key}-scrim")).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

pub fn settings(s: &App, p: Palette) -> El {
    let mut nav = col().key("settings-nav").w(160.0).h_full().px(8.0).py(10.0).gap(2.0).fill(p.sidebar);
    for (pane, name) in PANES {
        let on = s.pane == pane;
        nav = nav.child(row().key(format!("pane-{}", name.to_lowercase())).role("tab").w_full().h(28.0).px(10.0).center_y()
            .radius(6.0).fill(if on { p.sidebar_selection } else { Color::TRANSPARENT }).hover(p.hover)
            .on_press(move |s: &mut App| s.pane = pane)
            .child(text(name).font(if on { BODY_BOLD } else { BODY }).color(p.text)));
    }
    let title = PANES.iter().find(|(x, _)| *x == s.pane).map_or("", |(_, n)| n);
    let body = col().w_full().px(24.0).pt(16.0).pb(20.0).gap(2.0)
        .child(row().pb(6.0).child(text(title).font(BODY_BOLD).color(p.text)))
        .child(match s.pane {
            Pane::General => general(s, p),
            Pane::Accounts => accounts(s, p),
        });
    let bar = row().w_full().h(44.0).px(16.0).center_y()
        .child(text("Settings").font(BODY_BOLD).color(p.text).grow(1.0))
        .child(button("Done", p.on_selection, p.accent, p).default_button().on_press(|s: &mut App| s.close_sheet()));
    let card = col().key("settings").w(680.0).h(480.0).radius(12.0).clip_rounded(12.0)
        .fill(p.content).border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.close_sheet())
        .child(bar)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(row().w_full().grow(1.0).min_h(0.0)
            .child(nav)
            .child(el().w(1.0).h_full().fill(p.separator))
            .child(col().grow(1.0).min_w(0.0).h_full().scroll(ScrollSpec { stick_bottom: false }).child(body)));
    scrim("settings", card, p)
}

fn setting(label: &str, detail: Option<&str>, control: El, p: Palette) -> El {
    let mut words = col().grow(1.0).min_w(0.0).gap(2.0).child(text(label).font(BODY).color(p.text));
    if let Some(d) = detail {
        words = words.child(text(d).font(SMALL).color(p.secondary).wrap());
    }
    row().w_full().py(8.0).gap(16.0).center_y().child(words).child(control)
}

fn general(s: &App, p: Palette) -> El {
    let st = &s.settings;
    col().w_full()
        .child(setting("Opens in", None, segmented("set-view", &[("Day", View::Day), ("Week", View::Week), ("Month", View::Month)],
            st.view, |s, v| s.set_view(v), p), p))
        .child(setting("Weeks start on", None, segmented("set-week", &[("Monday", WeekStart::Monday), ("Sunday", WeekStart::Sunday)],
            st.week_start, |s, v| s.update_settings(|st| st.week_start = v), p), p))
        .child(setting("Day and Week open at", Some("Scrolled to this hour; scroll for the rest of the day."),
            segmented("set-starts", &[("6 AM", 6), ("7 AM", 7), ("8 AM", 8), ("9 AM", 9)],
                st.day_starts, |s, v| s.update_settings(|st| st.day_starts = v), p), p))
        .child(setting("Check for changes", None, segmented("set-refresh", &[("1 min", 1), ("5 min", 5), ("15 min", 15), ("Manually", 0)],
            st.refresh_minutes, |s, v| s.update_settings(|st| st.refresh_minutes = v), p), p))
        .child(setting("Appearance", None, segmented("set-appearance",
            &[("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)],
            st.appearance, |s, v| s.update_settings(|st| st.appearance = v), p), p))
}

fn accounts(s: &App, p: Palette) -> El {
    let mut c = col().w_full().gap(4.0)
        .child(text("The label is what the sidebar calls an account. Arrows set the sidebar's order.")
            .font(SMALL).color(p.tertiary).wrap());
    for (i, a) in s.accounts.iter().enumerate() {
        c = c.child(account_row(s, a, i, p));
    }
    let mut actions = row().w_full().pt(8.0).gap(8.0)
        .child(button("Add Account…", p.text, p.field, p).key("add-account").on_press(|s: &mut App| s.add_account()));
    if s.importable > 0 {
        actions = actions.child(button("Add from Rune Mail", p.text, p.field, p).key("import")
            .help("The Fastmail and Gmail calendars Rune Mail already signs in to")
            .on_press_tx(|s: &mut App, tx| s.import_rune_mail(tx)));
    }
    c.child(actions)
}

fn account_row(s: &App, a: &AccountState, index: usize, p: Palette) -> El {
    let id = a.account.id();
    let detail = match &a.status {
        Status::Failed(e) => e.clone(),
        Status::Connecting => format!("{} · {} · connecting…", a.account.provider().name(), a.account.user),
        Status::Ready => {
            let n = a.calendars.len();
            format!("{} · {} · {n} calendar{}", a.account.provider().name(), a.account.user, if n == 1 { "" } else { "s" })
        }
    };
    let confirming = s.confirm_remove.as_deref() == Some(id.as_str());
    let remove = {
        let id = id.clone();
        if confirming {
            button("Remove Account", Color::WHITE, p.today, p).on_press(move |s: &mut App| s.remove_account(&id))
        } else {
            button("Remove", p.text, p.field, p).on_press(move |s: &mut App| s.confirm_remove = Some(id.clone()))
        }
    };
    let (kl, kk, kc, kb) = (id.clone(), id.clone(), id.clone(), id.clone());
    let input = super::field::input(&a.label_edit, BODY, p).key(format!("label-{id}")).single_line().grow(1.0)
        .placeholder(&a.account.user, p.tertiary)
        .on_edit(move |s: &mut App| s.label_field(&kl))
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(s.label_field(&kk), e))
        .on_input_change(move |s: &mut App, _| s.save_label(&kc));
    let label = super::field::boxed(input, 28.0, 8.0, move |s| s.label_field(&kb), p);
    let (up, down) = (id.clone(), id.clone());
    let last = index + 1 == s.accounts.len();
    let order = row().gap(2.0)
        .child(arrow(&style::CHEVRON_UP, "Move up", index > 0, p).on_press(move |s: &mut App| s.move_account(&up, -1)))
        .child(arrow(&style::CHEVRON_DOWN, "Move down", !last, p).on_press(move |s: &mut App| s.move_account(&down, 1)));
    row().w_full().py(8.0).gap(8.0).center_y()
        .child(order)
        .child(col().grow(1.0).min_w(0.0).gap(4.0)
            .child(label)
            .child(text(detail).font(SMALL).color(p.secondary).truncate()))
        .child(remove)
}

fn arrow(i: &'static rune::Icon, help: &str, enabled: bool, p: Palette) -> El {
    el().size(22.0, 22.0).radius(5.0).center().hover(p.hover).help(help).opacity(if enabled { 1.0 } else { 0.3 })
        .child(icon(i, 12.0).color(p.secondary))
}

/// Where to get the app password each service needs.
fn help(provider: Provider) -> &'static str {
    match provider {
        Provider::Fastmail => "In Fastmail, open Settings → Privacy & Security → Manage app passwords, and create one \
            with access to Calendars (CalDAV).",
        Provider::Google => "Google needs an app password: turn on 2-Step Verification, then create one at \
            myaccount.google.com/apppasswords. This shows your primary calendar.",
        Provider::ICloud => "Sign in with your Apple Account address and an app-specific password from \
            account.apple.com → Sign-In and Security → App-Specific Passwords.",
        Provider::Other => "Your server's CalDAV address (often https://example.com/.well-known/caldav or the \
            address of your calendars), your user name and password.",
    }
}

pub fn add_account(s: &App, p: Palette) -> El {
    let f = &s.add;
    let mut card = col().key("add").w(460.0).padding(24.0).gap(12.0).radius(14.0).fill(p.content)
        .border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.close_sheet())
        .child(text("Add Account").font(style::BIG).color(p.text))
        .child(segmented("provider", &[("Fastmail", Provider::Fastmail), ("Google", Provider::Google), ("iCloud", Provider::ICloud), ("Other", Provider::Other)],
            f.provider, |s, v| {
                let user = s.add.user.text.clone();
                s.add = AddForm::new(v);
                s.add.user = TextEdit::new(&user);
                s.add.user.focused = true;
            }, p))
        .child(text(help(f.provider)).font(SMALL).color(p.secondary).wrap());
    if f.provider == Provider::Other {
        card = card.child(field(&f.server, "add-server", "https://dav.example.com/", false, |s| &mut s.add.server, p));
    }
    let user_hint = if f.provider == Provider::Other { "User name" } else { "Email address" };
    card = card
        .child(field(&f.user, "add-user", user_hint, false, |s| &mut s.add.user, p))
        .child(field(&f.password, "add-password", "App password", true, |s| &mut s.add.password, p))
        .child(text("Kept in your system keyring (or, without one, in ~/.config/rune-calendar, readable only by you).")
            .font(SMALL).color(p.tertiary).wrap());
    if let Some(e) = &f.error {
        card = card.child(text(e.clone()).font(SMALL).color(p.today).wrap());
    }
    let add = if f.busy {
        button("Connecting…", p.on_selection, p.accent.alpha(0.6), p)
    } else {
        button("Add", p.on_selection, p.accent, p).key("add-submit").default_button().on_press(|s: &mut App| s.submit_add())
    };
    card = card.child(row().w_full().gap(8.0).justify_end()
        .child(button("Cancel", p.text, p.field, p).on_press(|s: &mut App| s.close_sheet()))
        .child(add));
    scrim("add", card, p)
}

fn field(edit: &TextEdit, key: &str, placeholder: &str, secure: bool, pick: fn(&mut App) -> &mut TextEdit, p: Palette) -> El {
    let mut input = super::field::input(edit, BODY, p).key(key).single_line().grow(1.0)
        .placeholder(placeholder, p.tertiary)
        .on_edit(move |s: &mut App| pick(s))
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(pick(s), e))
        .on_submit(|s: &mut App, _| s.submit_add());
    if secure {
        input = input.secure();
    }
    super::field::boxed(input, 30.0, 10.0, pick, p)
}
