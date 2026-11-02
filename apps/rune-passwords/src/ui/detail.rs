//! The selected item, as Apple Passwords shows one: a card with its name and fields (a click
//! copies one, a website opens), a Security card for its password, and the same card with
//! fields to type in while editing.

use rune::{Color, El, TextEdit, Tx, col, el, icon, row, text};

use super::field::{input, platform_key};
use super::list::badge;
use super::style::{self, BODY, BODY_BOLD, CODE, MONO, NAME, Palette, SECTION, SMALL};
use super::{button, tool};
use crate::app::{App, CopyTip, Draft, Verdict};
use crate::strength::Strength;
use crate::totp::Totp;
use crate::vault::Item;

pub fn view(s: &App, p: Palette) -> El {
    let page = col().key("detail").grow(1.0).min_w(0.0).h_full().fill(p.content).scroll(rune::ScrollSpec { stick_bottom: false });
    let inner = col().w_full().max_w(640.0).padding(24.0).gap(20.0);
    let body = match (&s.draft, s.selected_item()) {
        (Some(d), _) => editing(s, d, p),
        (None, Some(item)) => {
            let mut c = inner.child(card_of(s, item, p));
            if let Some(v) = s.verdicts.get(&item.id) {
                c = c.child(security(*v, p));
            }
            c
        }
        (None, None) => return page.center().child(text("No Item Selected").font(BODY).color(p.secondary)),
    };
    page.child(row().w_full().justify_center().child(body))
}

/// A rounded card. Its rows run nearly to its edge, so a row's hover has room around its text;
/// what's in them lines up `INSET` in from the card's edge.
fn card(p: Palette) -> El {
    col().w_full().px(EDGE).py(8.0).radius(12.0).fill(p.card).border(1.0, p.separator)
}

const EDGE: f32 = 6.0;
const INSET: f32 = 16.0;
const PILL: f32 = 6.0;

/// Between rows: inset like their text, and clear of their hover.
fn rule(p: Palette) -> El {
    row().w_full().px(INSET - EDGE).py(2.0).child(el().grow(1.0).h(1.0).fill(p.separator))
}

/// A row of a card: its hover reaches near the card's edge, and its text sits `INSET` in.
/// Values sit in a `pill`, whose own padding makes up the rest on the right.
fn card_row() -> El {
    row().w_full().pl(INSET - EDGE).pr(INSET - EDGE - PILL).py(7.0).gap(12.0).center_y().radius(6.0)
}

/// "Last modified today at 9:41 AM".
fn modified(at: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(at) else { return String::new() };
    let t = t.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let day = match (today - t.date_naive()).num_days() {
        0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        _ => t.format("%-d %B %Y").to_string(),
    };
    format!("Last modified {day} at {}", t.format("%-I:%M %p"))
}

fn card_of(s: &App, item: &Item, p: Palette) -> El {
    let mut actions = row().gap(8.0);
    if item.deleted.is_some() {
        actions = actions
            .child(button("Recover", p.text, p.field, p).key("recover").on_press(|s: &mut App| s.restore_selected()))
            .child(button("Delete Now", p.danger, p.field, p).key("purge").on_press(|s: &mut App| s.ask_delete_selected()));
    } else if crate::app::can_edit(item) {
        actions = actions.child(button("Edit", p.text, p.field, p).key("edit").on_press(|s: &mut App| s.edit()));
    }
    let head = row().w_full().px(INSET - EDGE).pt(6.0).gap(14.0).center_y().pb(12.0)
        .child(badge(s, item, 56.0, p))
        .child(col().grow(1.0).min_w(0.0)
            .child(text(item.name.clone()).font(NAME).color(p.text).truncate())
            .child(text(modified(&item.revised)).font(SMALL).color(p.secondary)))
        .child(actions);
    let mut rows: Vec<El> = Vec::new();
    if !item.username.is_empty() {
        rows.push(field_row("User Name", text(item.username.clone()).font(BODY).color(p.text).truncate(), Some(("user name", item.username.clone())), p));
    }
    if !item.password.is_empty() {
        let shown = if s.revealed { text(item.password.clone()).font(MONO).color(p.text).truncate() } else { dots(p) };
        let eye = tool(&style::EYE, if s.revealed { "Hide" } else { "Show" }, p).key("reveal").on_press(|s: &mut App| s.revealed = !s.revealed);
        rows.push(field_row_with("Password", shown, Some(eye), Some(("password", item.password.clone())), p));
    }
    if let Some(t) = Totp::parse(&item.totp) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let code = t.code(now);
        let shown = if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code.clone() };
        let left = t.remaining(now);
        // The countdown has a fixed width too, so the value doesn't shift each second.
        let value = row().gap(8.0).center_y()
            .child(text(shown).font(CODE).color(p.text))
            .child(row().w(24.0).justify_end()
                .child(text(format!("{left}s")).font(SMALL.features("'tnum' 1")).color(if left <= 5 { p.danger } else { p.secondary })));
        rows.push(field_row("Verification Code", value, Some(("code", code)), p));
    } else if !item.totp.is_empty() {
        rows.push(field_row("Verification Code", text("Can't read this setup key").font(SMALL).color(p.secondary), None, p));
    }
    for site in &item.passkeys {
        rows.push(field_row("Passkey", text(site.clone()).font(BODY).color(p.text).truncate(), None, p));
    }
    for uri in &item.uris {
        let href = if uri.contains("://") { uri.clone() } else { format!("https://{uri}") };
        let shown = text(uri.clone()).font(BODY).color(p.accent).truncate();
        let link = pill(shown).key("link").hover(p.field).cursor(rune::Cursor::Pointer)
            .help(format!("Open {href}"))
            .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                if let Err(e) = tx.open_url(&href) {
                    s.say(format!("Couldn't open it: {e:?}"), tx);
                }
            });
        rows.push(card_row().key(format!("site-{uri}"))
            .child(text("Website").font(BODY).color(p.secondary).w(130.0))
            .child(row().grow(1.0).min_w(0.0).justify_end().child(link)));
    }
    for d in &item.details {
        let shown = if d.secret && !s.revealed { dots(p) } else { text(d.value.clone()).font(BODY).color(p.text).truncate() };
        rows.push(field_row(&d.label, shown, Some((d.label.as_str(), d.value.clone())), p));
    }
    if let Some(f) = item.folder.as_ref().and_then(|id| s.vault.as_ref()?.folders.iter().find(|f| &f.id == id)) {
        rows.push(field_row("Folder", text(f.name.clone()).font(BODY).color(p.text), None, p));
    }
    if let Some(o) = item.org.as_ref().and_then(|id| s.vault.as_ref()?.orgs.iter().find(|o| &o.id == id)) {
        rows.push(field_row("Shared in", text(o.name.clone()).font(BODY).color(p.text), None, p));
    }
    let mut c = card(p).child(head);
    for r in rows {
        c = c.child(rule(p)).child(r);
    }
    if !item.notes.is_empty() {
        c = c.child(rule(p)).child(text(item.notes.clone()).font(BODY).color(p.text).wrap().selectable(true).px(INSET - EDGE).py(8.0));
    }
    c
}

fn dots(p: Palette) -> El {
    text("•".repeat(14)).font(BODY_BOLD).color(p.text)
}

/// A label on the left, the value on the right; a click copies `copy`.
/// A label on the left, the value on the right; a click on a value that copies copies it. Over
/// one, the pointer is a hand, the value lights up, and "Click to copy" shows above it.
fn field_row(label: &str, value: El, copy: Option<(&str, String)>, p: Palette) -> El {
    field_row_with(label, value, None, copy, p)
}

/// `field_row` with a control after the value (the password's eye), outside what's copied.
fn field_row_with(label: &str, value: El, after: Option<El>, copy: Option<(&str, String)>, p: Palette) -> El {
    let key = format!("field-{label}");
    let mut value = pill(value);
    if let Some((what, v)) = copy {
        let what = what.to_lowercase();
        let (enter, rect) = (key.clone(), key.clone());
        value = value.key(format!("{key}-value")).hover(p.field).cursor(rune::Cursor::Pointer)
            .on_hover_change(move |s: &mut App, inside, _: &mut Tx| {
                if inside {
                    s.copy_tip = Some(CopyTip { key: enter.clone() });
                } else if s.copy_tip.as_ref().is_some_and(|t| t.key == enter) {
                    s.copy_tip = None;
                }
            })
            // Where the value is, for placing "Click to copy": only worth a view run when the tip
            // shows over this one (a value moving otherwise would re-run the whole view).
            .on_drawn_layout_tx(move |s: &mut App, r, tx: &mut Tx| {
                s.field_rects.insert(rect.clone(), r);
                if !s.copy_tip.as_ref().is_some_and(|t| t.key == rect) {
                    tx.live_only();
                }
            })
            .on_press_tx(move |s: &mut App, tx: &mut Tx| s.copy(&what, v.clone(), tx));
    }
    let mut end = row().grow(1.0).min_w(0.0).justify_end().center_y().gap(2.0).child(value);
    if let Some(a) = after {
        end = end.child(a);
    }
    card_row().key(key).child(text(label.to_owned()).font(BODY).color(p.secondary).w(130.0)).child(end)
}

/// A value with room for a soft highlight around it.
fn pill(value: El) -> El {
    row().min_w(0.0).px(PILL).py(2.0).radius(5.0).center_y().child(value)
}

fn security(v: Verdict, p: Palette) -> El {
    let (glyph, colour, title, says) = if v.reused > 0 {
        let others = if v.reused == 1 { "1 other item".to_owned() } else { format!("{} other items", v.reused) };
        (&*style::SHIELD, *style::ORANGE, "Reused Password", format!("This password is also used for {others}. Change it to one that's unique."))
    } else {
        match v.strength {
            Strength::Weak => (&*style::SHIELD, *style::RED, "Weak Password", "This password is easy to guess. Change it to a strong one.".to_owned()),
            Strength::Fair => (&*style::SHIELD_OK, *style::YELLOW, "Fair Password", "This password could be longer or less predictable.".to_owned()),
            Strength::Strong => (&*style::SHIELD_OK, *style::GREEN, "Strong Password", "This password is long, difficult to guess, and unique.".to_owned()),
        }
    };
    col().w_full().gap(8.0)
        .child(text("SECURITY").font(SECTION).color(p.tertiary).px(4.0))
        .child(card(p).items_center().gap(6.0).py(18.0)
            .child(icon(glyph, 28.0).color(colour))
            .child(text(title).font(BODY_BOLD).color(p.text))
            .child(text(says).font(SMALL).color(p.secondary).wrap().align(rune::Align::Center)))
}

/// The item's fields to type in, with Save and Cancel; and Delete for one already saved.
fn editing(s: &App, d: &Draft, p: Palette) -> El {
    let saving = s.busy;
    let save = button(if saving { "Saving…" } else { "Save" }, Color::WHITE, p.accent, p).key("save");
    let head = row().w_full().px(INSET - EDGE).pt(6.0).gap(14.0).center_y().pb(12.0)
        .child(badge(s, &d.item(), 56.0, p))
        .child(edit_field(&d.name, "name", "Name", false, |d| &mut d.name, p).grow(1.0))
        .child(button("Cancel", p.text, p.field, p).key("cancel").on_press(|s: &mut App| {
            s.cancel_edit();
        }))
        .child(if saving { save } else { save.on_press_tx(|s: &mut App, tx: &mut Tx| s.save(tx)) });
    let password = row().w_full().gap(6.0).center_y()
        .child(edit_field(&d.password, "password", "Password", false, |d| &mut d.password, p).grow(1.0))
        .child(tool(&style::SPARKLES, "Suggest a strong password", p).key("generate").on_press(|s: &mut App| s.generate_password()));
    let mut c = card(p).child(head)
        .child(rule(p)).child(labelled("User Name", edit_field(&d.username, "username", "Email or user name", false, |d| &mut d.username, p), p))
        .child(rule(p)).child(labelled("Password", password, p))
        .child(rule(p)).child(labelled("Websites", edit_field(&d.websites, "websites", "example.com (one per line)", true, |d| &mut d.websites, p), p))
        .child(rule(p)).child(labelled("Verification Code", edit_field(&d.code, "code", "Setup key or otpauth:// link", false, |d| &mut d.code, p), p))
        .child(rule(p)).child(labelled("Notes", edit_field(&d.notes, "notes", "Add a note", true, |d| &mut d.notes, p), p));
    if !d.item.id.is_empty() {
        let id = d.item.id.clone();
        c = c.child(rule(p)).child(row().w_full().px(INSET - EDGE).pt(8.0).pb(6.0)
            .child(button("Delete", p.danger, p.field, p).key("delete").on_press(move |s: &mut App| s.ask_delete(&id))));
    }
    col().w_full().max_w(640.0).padding(24.0).child(c)
}

fn labelled(label: &str, field: El, p: Palette) -> El {
    row().w_full().px(INSET - EDGE).py(6.0).gap(12.0).items_start()
        .child(text(label.to_owned()).font(BODY).color(p.secondary).w(130.0).pt(5.0))
        .child(field.grow(1.0).min_w(0.0))
}

/// One of the draft's fields, picked out by `pick`.
fn edit_field(edit: &TextEdit, key: &str, hint: &str, multi: bool, pick: fn(&mut Draft) -> &mut TextEdit, p: Palette) -> El {
    let field = lens(move |s: &mut App| pick(s.draft.as_mut().expect("a field of the draft is shown only while there's one")));
    let mut i = input(edit, BODY, p).key(format!("edit-{key}")).grow(1.0).min_w(0.0).placeholder(hint, p.tertiary)
        .on_edit(field)
        .on_input_key(move |s: &mut App, e: &rune::KeyEvent, tx: &mut Tx| {
            if s.draft.is_none() {
                return false;
            }
            if e.key == rune::Key::Enter && !e.mods.shift && !multi {
                s.save(tx);
                return true;
            }
            platform_key(field(s), e)
        });
    i = if multi { i.wrap() } else { i.single_line() };
    row().w_full().min_h(28.0).px(8.0).py(4.0).radius(6.0).fill(p.field).center_y().cursor(rune::Cursor::Text)
        .on_press(move |s: &mut App| {
            if s.draft.is_some() {
                field(s).focused = true;
            }
        })
        .child(i)
}

/// Pins a closure's signature to the one handlers take: a field borrowed from the app as long as
/// the app is.
fn lens(f: impl Fn(&mut App) -> &mut TextEdit + Copy + 'static) -> impl Fn(&mut App) -> &mut TextEdit + Copy + 'static {
    f
}
