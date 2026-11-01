//! The compose sheet: To, Cc and Subject over a plain-text body.

use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, icon, row, text};

use super::field;
use super::style::{BODY, BODY_BOLD, Palette, READER};
use crate::app::{AddressField, App, Compose};

pub fn view(s: &App, p: Palette) -> El {
    let c = &s.compose;
    let title = if c.subject.text.trim().is_empty() { "New Message".to_owned() } else { c.subject.text.clone() };
    let send_label = if c.sending { "Sending…" } else { "Send" };
    let bar = row().w_full().h(44.0).px(12.0).gap(8.0).center_y()
        .child(button("Cancel", p.text, p.field, p).on_press(|s: &mut App| s.close_sheet()))
        .child(text(title).font(BODY_BOLD).color(p.text).grow(1.0).min_w(0.0).truncate().align(rune::Align::Center))
        .child(el().key("attach").size(30.0, 26.0).radius(6.0).center().hover(p.hover).help("Attach a file (or drop files here)")
            .on_press_tx(|s: &mut App, tx| s.pick_attachment(tx))
            .child(icon(&super::style::CLIP, 15.0).color(p.secondary)))
        .child(button(send_label, p.on_selection, p.accent, p).help("Send (Ctrl+Enter)")
            .on_press_tx(|s: &mut App, tx| s.send_compose(tx)));
    // A plain-paragraph field submits on Return, as a chat composer does; mail wants a new
    // line, so Return types one and Ctrl+Return sends.
    let body = field::input(&c.body, READER, p).key("compose-body").w_full().grow(1.0).min_h(0.0).px(16.0).py(12.0)
        .on_edit(|s: &mut App| &mut s.compose.body)
        .on_input_key(|s: &mut App, e: &KeyEvent, tx: &mut Tx| match e.key {
            _ if field::platform_key(&mut s.compose.body, e) => true,
            Key::Enter if e.mods.cmd => {
                s.send_compose(tx);
                true
            }
            Key::Enter if !e.mods.alt && !e.mods.shift => {
                s.compose.body.insert("\n");
                true
            }
            _ => false,
        })
        // A pasted image goes in at the caret, as a pill; text pastes as usual.
        .on_input_paste(|s: &mut App, content, _| {
            let Some(image) = content.image() else { return false };
            s.paste_image(image.png().to_vec());
            true
        })
        .pill_style(|id: &str| {
            if id.starts_with("image:") { rune::PillStyle::new().icon(&super::style::IMAGE) } else { rune::PillStyle::new() }
        });
    let mut sheet = col().key("compose").w(660.0).h(540.0).radius(12.0).clip_rounded(12.0)
        .fill(if c.drop_over { p.accent.alpha(0.06).mix(p.content, 0.5) } else { p.content })
        .border(if c.drop_over { 2.0 } else { 1.0 }, if c.drop_over { p.accent } else { p.separator })
        .shadow(16.0, 48.0, rune::Color::BLACK.alpha(0.3))
        .modal(false, |s: &mut App| s.close_sheet())
        // Files dropped anywhere on the sheet attach.
        .on_drop_files(|s: &mut App, files, tx| {
            s.compose.drop_over = false;
            for f in files {
                s.attach_file(f, tx);
            }
        })
        .on_drop_hover(|s: &mut App, over, _| s.compose.drop_over = over)
        .child(bar)
        .child(divider(p));
    if s.accounts.len() > 1 {
        sheet = sheet.child(from(s, p)).child(divider(p));
    }
    let sheet = sheet
        .child(field("To:", &c.to, "compose-to", |c| &mut c.to, |c| &mut c.cc, Some(AddressField::To), p))
        .child(divider(p))
        .child(field("Cc:", &c.cc, "compose-cc", |c| &mut c.cc, |c| &mut c.subject, Some(AddressField::Cc), p))
        .child(divider(p))
        .child(field("Subject:", &c.subject, "compose-subject", |c| &mut c.subject, |c| &mut c.body, None, p))
        .child(divider(p))
        .child(body)
        .children(attachments(s, p));
    el().key("compose-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(sheet)
}

/// The strip of attachments under the body: a thumbnail or paperclip, name, size, remove.
fn attachments(s: &App, p: Palette) -> Option<El> {
    let list = &s.compose.attachments;
    if list.is_empty() {
        return None;
    }
    let mut strip = row().key("attachments").w_full().px(12.0).py(8.0).gap(8.0).fill(p.reader_bg);
    for (i, a) in list.iter().enumerate() {
        // Fitted inside the square at its own shape, once decoded (the view runs again then).
        let thumb = match a.thumb.as_ref().and_then(|t| t.size().map(|sz| (t, sz))) {
            Some((img, [w, h])) => {
                let scale = 28.0 / (w.max(h).max(1) as f32);
                el().size(28.0, 28.0).radius(4.0).fill(p.field).center()
                    .child(el().size(w as f32 * scale, h as f32 * scale).pixels(img.pixels()))
            }
            None => el().size(28.0, 28.0).radius(4.0).fill(p.field).center()
                .child(icon(if a.is_image() { &super::style::IMAGE } else { &super::style::CLIP }, 14.0).color(p.secondary)),
        };
        let label = if a.cid.is_some() { format!("{} (in the message)", a.name) } else { a.name.clone() };
        strip = strip.child(row().key(format!("attachment-{i}")).h(36.0).pl(4.0).pr(4.0).gap(8.0).center_y().radius(8.0)
            .fill(p.card).border(1.0, p.separator).max_w(240.0)
            .help(format!("{} · {}", a.name, a.size_label()))
            .child(thumb)
            .child(col().min_w(0.0).shrink()
                .child(text(label).font(super::style::SMALL).color(p.text).truncate())
                .child(text(a.size_label()).font(super::style::COUNT).color(p.secondary)))
            .child(el().size(20.0, 20.0).radius(10.0).center().hover(p.hover).help("Remove")
                .on_press(move |s: &mut App| s.remove_attachment(i))
                .child(text("×").font(BODY).color(p.secondary))));
    }
    Some(col().w_full().child(divider(p)).child(strip.scroll_x()))
}

fn account_title(a: &crate::app::Account) -> String {
    match &a.label {
        Some(l) => format!("{l} <{}>", a.key),
        None => a.key.clone(),
    }
}

/// Which account sends: a select showing the current one. It opens the system's pop-up menu
/// where there is one (macOS), else a list drawn under it.
fn from(s: &App, p: Palette) -> El {
    let current = s.account(&s.compose.account).map_or_else(|| s.compose.account.clone(), account_title);
    let select = row().key("compose-from-select").role("combobox").h(26.0).px(10.0).gap(8.0).center_y().radius(6.0)
        .fill(p.field).hover(p.hover).border(1.0, if s.from_open { p.accent } else { rune::Color::TRANSPARENT })
        .on_press_tx(|s: &mut App, tx: &mut Tx| {
            if rune::popup_menu_supported() {
                let options: Vec<(String, String)> = s.accounts.iter().map(|a| (a.key.clone(), account_title(a))).collect();
                let menu = rune::PopupMenu::picker(options, &s.compose.account);
                tx.popup_menu(menu, "compose-from-select", |s: &mut App, picked, _| {
                    if let Some(key) = picked {
                        s.compose.account = key.to_owned();
                    }
                });
            } else {
                s.from_open = !s.from_open;
            }
        })
        .child(text(current).font(BODY).color(p.text).truncate().min_w(0.0))
        .child(icon(&super::style::CHEVRON_DOWN, 12.0).color(p.secondary));
    row().key("compose-from").w_full().h(36.0).px(16.0).gap(8.0).center_y()
        .child(text("From:").font(BODY).color(p.secondary))
        .child(select)
}

/// The From select's list, where there's no native pop-up menu: every account, the current
/// one checked. Drawn at the window root so it lies over the fields below.
pub fn from_menu(s: &App, p: Palette) -> Option<El> {
    if !s.from_open {
        return None;
    }
    let mut menu = col().key("from-menu").min_w(300.0).padding(4.0).gap(1.0).radius(8.0)
        .fill(p.content).border(1.0, p.separator).shadow(6.0, 18.0, rune::Color::BLACK.alpha(0.2));
    for a in &s.accounts {
        let on = a.key == s.compose.account;
        let key = a.key.clone();
        menu = menu.child(row().key(format!("from-{}", a.key)).w_full().px(8.0).py(6.0).gap(8.0).center_y().radius(5.0)
            .hover(p.hover)
            .on_press(move |s: &mut App| {
                s.compose.account = key.clone();
                s.from_open = false;
            })
            .child(el().w(14.0).child(if on { icon(&super::style::CHECK, 12.0).color(p.accent) } else { el() }))
            .child(text(account_title(a)).font(BODY).color(p.text).truncate().min_w(0.0)));
    }
    Some(menu.anchored("compose-from-select", rune::Side::Bottom, rune::Align::Start, 4.0))
}

type Pick = fn(&mut Compose) -> &mut TextEdit;

fn field(label: &str, edit: &TextEdit, key: &str, pick: Pick, next: Pick, address: Option<AddressField>, p: Palette) -> El {
    let key_is_subject = key == "compose-subject";
    // Return in a header field moves on to the next one, unless it's taking a suggestion.
    row().key(format!("{key}-row")).w_full().h(34.0).px(16.0).gap(8.0).center_y()
        .child(text(label).font(BODY).color(p.secondary))
        .child(field::input(edit, BODY, p).key(key).single_line().grow(1.0)
            .on_edit(move |s: &mut App| pick(&mut s.compose))
            .on_input_key(move |s: &mut App, e: &KeyEvent, tx: &mut Tx| {
                // Ctrl+Return sends from any field (the field's own Return moves on).
                if e.key == Key::Enter && e.mods.cmd {
                    s.send_compose(tx);
                    return true;
                }
                // Tab from Subject goes to the top of the message, where a reply is written
                // (Tab's own move would leave the caret at the end, under the quote).
                if key_is_subject && e.key == Key::Tab && !e.mods.shift && !e.mods.cmd {
                    pick(&mut s.compose).focused = false;
                    s.compose.body.set_caret(0);
                    s.compose.body.focused = true;
                    return true;
                }
                if let Some(f) = address
                    && let Some((open, list)) = s.open_suggestions()
                    && open == f
                {
                    let last = list.len() - 1;
                    match e.key {
                        Key::Down => s.suggest_index = (s.suggest_index + 1).min(last),
                        Key::Up => s.suggest_index = s.suggest_index.saturating_sub(1),
                        // Enter takes it and stays, for another address; Tab takes it and moves on.
                        Key::Enter if !e.mods.cmd && !e.mods.shift => s.accept_suggestion(f, &list[s.suggest_index.min(last)]),
                        Key::Tab if !e.mods.cmd && !e.mods.shift => {
                            s.accept_suggestion(f, &list[s.suggest_index.min(last)]);
                            pick(&mut s.compose).focused = false;
                            next(&mut s.compose).focused = true;
                        }
                        Key::Escape => s.suggest_hidden = true,
                        _ => return field::platform_key(pick(&mut s.compose), e),
                    }
                    return true;
                }
                field::platform_key(pick(&mut s.compose), e)
            })
            .on_input_change(|s: &mut App, _| {
                s.suggest_index = 0;
                s.suggest_hidden = false;
            })
            .on_submit(move |s: &mut App, tx| {
                if tx.mods().cmd {
                    s.send_compose(tx);
                    return;
                }
                if key_is_subject {
                    s.compose.body.set_caret(0);
                }
                next(&mut s.compose).focused = true;
            }))
}

fn divider(p: Palette) -> El {
    el().w_full().h(1.0).fill(p.separator)
}

pub fn button(label: &str, ink: rune::Color, fill: rune::Color, p: Palette) -> El {
    row().h(26.0).px(12.0).radius(6.0).center().fill(fill).hover(p.hover)
        .child(text(label).font(BODY).color(ink))
}

/// The address suggestions under the To or Cc field being typed in: name and address, the
/// highlighted one in the accent colour. Drawn at the window root so it lies over the
/// fields below.
pub fn suggestions(s: &App, p: Palette) -> Option<El> {
    let (field, list) = s.open_suggestions()?;
    let anchor = match field {
        AddressField::To => "compose-to",
        AddressField::Cc => "compose-cc",
    };
    let mut menu = col().key("suggestions").w(380.0).padding(4.0).gap(1.0).radius(8.0)
        .fill(p.content).border(1.0, p.separator).shadow(6.0, 18.0, rune::Color::BLACK.alpha(0.2));
    for (i, sug) in list.iter().enumerate() {
        let on = i == s.suggest_index.min(list.len() - 1);
        let (ink, muted) = if on { (p.on_selection, p.on_selection.alpha(0.8)) } else { (p.text, p.secondary) };
        let pick = sug.clone();
        let mut line = row().w_full().gap(8.0).center_y()
            .child(text(if sug.name.is_empty() { sug.email.clone() } else { sug.name.clone() }).font(BODY).color(ink).truncate().min_w(0.0));
        if !sug.name.is_empty() {
            line = line.child(text(sug.email.clone()).font(super::style::SMALL).color(muted).grow(1.0).min_w(0.0).truncate());
        }
        menu = menu.child(col().key(format!("suggest-{i}")).w_full().px(10.0).py(6.0).radius(5.0)
            .fill(if on { p.accent } else { rune::Color::TRANSPARENT })
            .hover(if on { rune::Color::TRANSPARENT } else { p.hover })
            .help(if sug.contact { "From your contacts" } else { "From your mail" })
            .on_press(move |s: &mut App| s.accept_suggestion(field, &pick))
            .child(line));
    }
    Some(menu.anchored(anchor, rune::Side::Bottom, rune::Align::Start, 8.0))
}
