//! The sidebar: Apple Passwords' category tiles, then folders and the organisations items are
//! shared through, then lock and settings. And the settings sheet.

use rune::{Color, El, Icon, Tx, col, el, icon, row, text};

use super::style::{self, BODY, BODY_BOLD, COUNT, Palette, SECTION, SMALL, TILE_LABEL};
use super::{button, segmented, tool};
use crate::app::{AccountEntry, App, Category};
use crate::settings::Appearance;

pub const WIDTH: f32 = 250.0;

pub fn view(s: &App, p: Palette) -> El {
    let tiles: [(Category, &'static Icon, Color); 6] = [
        (Category::All, &style::KEY, *style::BLUE),
        (Category::Passkeys, &style::PERSON, *style::GREEN),
        (Category::Codes, &style::CLOCK, *style::YELLOW),
        (Category::Notes, &style::NOTE, *style::PURPLE),
        (Category::Security, &style::SHIELD, *style::RED),
        (Category::Deleted, &style::TRASH, *style::ORANGE),
    ];
    let mut grid = col().w_full().gap(8.0);
    for pair in tiles.chunks(2) {
        let mut r = row().w_full().gap(8.0);
        for (c, i, colour) in pair {
            r = r.child(tile(s, c.clone(), i, *colour, p));
        }
        grid = grid.child(r);
    }
    // Only the folders and groups scroll; the tiles above and the toolbar and account below
    // stay put.
    let mut c = col().key("sidebar-scroll").w_full().grow(1.0).min_h(0.0).pb(8.0).px(12.0).gap(4.0)
        .scroll(rune::ScrollSpec { stick_bottom: false });
    if let Some(v) = &s.vault {
        if !v.folders.is_empty() {
            c = c.child(heading("FOLDERS", p));
            for f in &v.folders {
                c = c.child(row_item(s, Category::Folder(f.id.clone()), &style::FOLDER, &f.name, p));
            }
        }
        if !v.orgs.is_empty() {
            c = c.child(heading("SHARED GROUPS", p));
            for o in &v.orgs {
                c = c.child(row_item(s, Category::Org(o.id.clone()), &style::PEOPLE, &o.name, p));
            }
        }
        if v.unreadable > 0 {
            c = c.child(text(format!("{} items couldn't be opened with this account's keys.", v.unreadable))
                .font(SMALL).color(p.tertiary).wrap().px(6.0).pt(8.0));
        }
    }
    let mut foot = row().w_full().gap(4.0).center_y().pt(6.0).pb(6.0)
        .child(tool(&style::LOCK, "Lock (Ctrl+L)", p).key("lock").on_press(|s: &mut App| s.lock()))
        .child(tool(&style::REFRESH, "Sync (Ctrl+R)", p).key("sync").on_press(|s: &mut App| s.sync()))
        .child(el().grow(1.0));
    if s.offline {
        foot = foot.child(text("Offline").font(SMALL).color(p.tertiary));
    }
    foot = foot.child(tool(&style::GEAR, "Settings", p).key("settings").on_press(|s: &mut App| s.settings_open = true));
    col().key("sidebar").w(WIDTH).h_full().fill(p.sidebar)
        .child(col().w_full().pt(16.0).px(12.0).pb(4.0).child(grid))
        .child(c)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(col().w_full().px(12.0).child(foot).child(account_select(s, p)))
}

/// The bottom left: the account in use, and a press opens the menu of the others.
fn account_select(s: &App, p: Palette) -> El {
    let (email, host) = s.current_account().map_or((s.email.clone(), String::new()), |a| (a.email.clone(), a.host.clone()));
    let mut who = col().grow(1.0).min_w(0.0).child(text(email.clone()).font(BODY_BOLD).color(p.text).truncate());
    if !host.is_empty() {
        who = who.child(text(host).font(SMALL).color(p.secondary).truncate());
    }
    row().key("account").role("button").label(format!("Account: {email}")).help("Switch or add an account")
        .w_full().h(44.0).px(8.0).mb(10.0).gap(8.0).center_y().radius(8.0)
        .fill(if s.accounts_open { p.sidebar_selection } else { Color::TRANSPARENT })
        .hover(p.hover)
        .on_press(|s: &mut App| s.accounts_open = !s.accounts_open)
        .child(avatar(&email, 28.0))
        .child(who)
        .child(icon(&style::CHEVRONS, 14.0).color(p.secondary))
}

/// A round tile with the address's first letter.
pub fn avatar(email: &str, size: f32) -> El {
    let letter = email.chars().next().map_or("?".to_owned(), |c| c.to_uppercase().to_string());
    el().size(size, size).radius(size / 2.0).center().fill(style::tile_colour(email))
        .child(text(letter).font(rune::TextStyle::new(size * 0.46, 600.0)).color(Color::WHITE))
}

/// A kept account as a row: its letter, address and server.
pub fn account_row(a: &AccountEntry, current: bool, p: Palette) -> El {
    row().key(format!("account-{}", a.id)).w_full().h(40.0).px(8.0).gap(8.0).center_y().radius(6.0).hover(p.hover)
        .child(avatar(&a.email, 24.0))
        .child(col().grow(1.0).min_w(0.0)
            .child(text(a.email.clone()).font(BODY).color(p.text).truncate())
            .child(text(a.host.clone()).font(SMALL).color(p.secondary).truncate()))
        .child(el().size(16.0, 16.0).center().child(icon(&style::CHECK, 14.0).color(p.accent).opacity(if current { 1.0 } else { 0.0 })))
}

/// The account menu, above the sidebar's account: every kept account, then Add Account.
pub fn account_menu(s: &App, p: Palette) -> El {
    let mut menu = col().key("account-menu").role("menu").pin(Some(12.0), None, None, Some(58.0)).w(WIDTH - 24.0)
        .padding(4.0).gap(2.0).radius(10.0).fill(p.content).border(1.0, p.separator)
        .shadow(8.0, 28.0, Color::BLACK.alpha(if p.dark { 0.45 } else { 0.18 }))
        .modal(true, |s: &mut App| s.accounts_open = false);
    for a in &s.accounts {
        let id = a.id.clone();
        let current = s.account_id.as_deref() == Some(a.id.as_str());
        menu = menu.child(account_row(a, current, p).role("menuitem").on_press(move |s: &mut App| s.switch_account(&id)));
    }
    menu.child(el().w_full().h(1.0).fill(p.separator).mx(4.0).my(2.0))
        .child(row().key("add-account").role("menuitem").w_full().h(32.0).px(8.0).gap(8.0).center_y().radius(6.0).hover(p.hover)
            .on_press(|s: &mut App| s.add_account())
            .child(el().size(24.0, 24.0).center().child(icon(&style::PLUS, 15.0).color(p.secondary)))
            .child(text("Add Account…").font(BODY).color(p.text)))
}

fn tile(s: &App, c: Category, i: &'static Icon, colour: Color, p: Palette) -> El {
    let on = s.category == c;
    let count = s.count(&c);
    let label = c.title(None);
    let (ink, fill, badge, glyph) = if on {
        (p.on_selection, p.accent, Color::WHITE, p.accent)
    } else {
        (p.text, p.tile, colour, Color::WHITE)
    };
    col().key(format!("tile-{}", label.to_lowercase())).grow(1.0).h(62.0).padding(8.0).radius(10.0).fill(fill)
        .hover(if on { Color::TRANSPARENT } else { p.hover })
        .on_press(move |s: &mut App| s.choose(c.clone()))
        .child(row().w_full().center_y()
            .child(el().size(26.0, 26.0).radius(13.0).center().fill(badge).child(icon(i, 14.0).color(glyph)))
            .child(el().grow(1.0))
            .child(text(count.to_string()).font(COUNT).color(ink)))
        .child(el().grow(1.0))
        .child(text(label).font(TILE_LABEL).color(ink))
}

fn heading(label: &str, p: Palette) -> El {
    text(label.to_owned()).font(SECTION).color(p.tertiary).px(6.0).pt(14.0).pb(4.0)
}

fn row_item(s: &App, c: Category, i: &'static Icon, name: &str, p: Palette) -> El {
    let on = s.category == c;
    let count = s.count(&c);
    row().key(format!("cat-{name}")).w_full().h(28.0).px(8.0).gap(8.0).center_y().radius(6.0)
        .fill(if on { p.sidebar_selection } else { Color::TRANSPARENT })
        .hover(p.hover)
        .on_press(move |s: &mut App| s.choose(c.clone()))
        .child(icon(i, 15.0).color(p.secondary))
        .child(text(name.to_owned()).font(BODY).color(p.text).grow(1.0).min_w(0.0).truncate())
        .child(text(count.to_string()).font(SMALL).color(p.secondary))
}

pub fn settings(s: &App, p: Palette) -> El {
    let lock_options = [("5 min", 5), ("15 min", 15), ("1 hour", 60), ("Never", 0)];
    let card = col().key("settings-sheet").w(420.0).padding(24.0).gap(14.0).radius(14.0).fill(p.content)
        .border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.settings_open = false)
        .child(text("Settings").font(style::BIG).color(p.text))
        .child(setting("Appearance", segmented("appearance", &[("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)],
            s.settings.appearance, |s: &mut App, v, tx: &mut Tx| s.update_settings(|st| st.appearance = v, tx), p), p))
        .child(setting("Lock after", segmented("lock-after", &lock_options, s.settings.lock_minutes,
            |s: &mut App, v, tx: &mut Tx| s.update_settings(|st| st.lock_minutes = v, tx), p), p))
        .child(text(format!("Signed in as {}. Sign Out removes this account from this computer.", s.email)).font(SMALL).color(p.secondary).wrap())
        .child(row().w_full().gap(8.0)
            .child(button("Sign Out", p.danger, p.field, p).key("sign-out").on_press(|s: &mut App| s.sign_out()))
            .child(el().grow(1.0))
            .child(button("Done", Color::WHITE, p.accent, p).key("done").on_press(|s: &mut App| s.settings_open = false)));
    el().w_full().h_full().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

fn setting(label: &str, control: El, p: Palette) -> El {
    row().w_full().gap(12.0).center_y()
        .child(text(label.to_owned()).font(BODY).color(p.text).w(90.0))
        .child(control)
}
