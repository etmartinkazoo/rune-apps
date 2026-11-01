//! The Settings and Keyboard Shortcuts sheets.

use rune::{Color, El, ScrollSpec, col, el, row, text};

use super::compose::button;
use super::keys::{SHORTCUTS, VIM};
use super::style::{self, BODY, BODY_BOLD, Palette, SECTION, SMALL};
use crate::app::{App, SettingsPane};
use crate::backend::Kind;
use crate::settings::{Appearance, Settings, SwipeLeft};

fn sheet(key: &str, title: &str, w: f32, body: El, p: Palette) -> El {
    let bar = row().w_full().h(44.0).px(16.0).center_y()
        .child(text(title).font(BODY_BOLD).color(p.text).grow(1.0))
        .child(button("Done", p.on_selection, p.accent, p).default_button().on_press(|s: &mut App| s.close_sheet()));
    let card = col().key(key).w(w).max_h(620.0).radius(12.0).clip_rounded(12.0)
        .fill(p.content).border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.close_sheet())
        .child(bar)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(col().w_full().shrink().min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(body));
    el().key(format!("{key}-scrim")).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

fn section(title: &str, p: Palette) -> El {
    row().w_full().pt(14.0).pb(6.0).child(text(title.to_uppercase()).font(SECTION).color(p.secondary))
}

fn setting(label: &str, detail: Option<&str>, control: El, p: Palette) -> El {
    let mut words = col().grow(1.0).min_w(0.0).gap(2.0).child(text(label).font(BODY).color(p.text));
    if let Some(d) = detail {
        words = words.child(text(d).font(SMALL).color(p.secondary).wrap());
    }
    row().w_full().py(8.0).gap(16.0).center_y().child(words).child(control)
}

fn switch(key: &str, on: bool, set: fn(&mut Settings, bool), p: Palette) -> El {
    let knob = el().size(16.0, 16.0).radius(8.0).fill(Color::WHITE).shadow(1.0, 2.0, Color::BLACK.alpha(0.25))
        .absolute(if on { 16.0 } else { 2.0 }, 2.0).motion(rune::tok::motion::SNAPPY);
    el().key(key.to_owned()).role("switch").focusable(true).size(34.0, 20.0).radius(10.0)
        .fill(if on { p.accent } else { p.field.alpha(0.25).mix(p.secondary, 0.5) })
        .motion(rune::tok::motion::SNAPPY)
        .on_press_tx(move |s: &mut App, tx| s.update_settings(|st| set(st, !on), tx))
        .child(knob)
}

fn segmented<T: Copy + PartialEq + 'static>(
    key: &str,
    options: &[(&str, T)],
    current: T,
    set: fn(&mut Settings, T),
    p: Palette,
) -> El {
    let mut r = row().key(key.to_owned()).role("radiogroup").h(26.0).padding(2.0).gap(2.0).radius(7.0).fill(p.field);
    for (label, value) in options {
        let (label, value) = (*label, *value);
        let on = value == current;
        r = r.child(
            row().h(22.0).px(10.0).center().radius(5.0)
                .fill(if on { p.content } else { Color::TRANSPARENT })
                .hover(p.hover)
                .on_press_tx(move |s: &mut App, tx| s.update_settings(|st| set(st, value), tx))
                .child(text(label).font(SMALL).color(if on { p.text } else { p.secondary })),
        );
    }
    r
}

/// The panes, in the sidebar's order.
const PANES: [(SettingsPane, &str); 7] = [
    (SettingsPane::General, "General"),
    (SettingsPane::Viewing, "Viewing"),
    (SettingsPane::Accounts, "Accounts"),
    (SettingsPane::Privacy, "Privacy"),
    (SettingsPane::Storage, "Storage"),
    (SettingsPane::Ai, "Copy for AI"),
    (SettingsPane::Keyboard, "Keyboard"),
];

/// Settings: a sidebar of panes on the left, the chosen one on the right.
pub fn settings(s: &App, p: Palette) -> El {
    let mut nav = col().key("settings-nav").w(172.0).h_full().px(8.0).py(10.0).gap(2.0).fill(p.sidebar);
    for (pane, name) in PANES {
        let on = s.settings_pane == pane;
        nav = nav.child(
            row().key(format!("pane-{}", name.to_lowercase().replace(' ', "-"))).role("tab").w_full().h(28.0).px(10.0)
                .center_y().radius(6.0)
                .fill(if on { p.sidebar_selection } else { Color::TRANSPARENT })
                .hover(p.hover)
                .on_press(move |s: &mut App| s.settings_pane = pane)
                .child(text(name).font(if on { BODY_BOLD } else { BODY }).color(p.text)),
        );
    }
    let title = PANES.iter().find(|(pane, _)| *pane == s.settings_pane).map_or("Settings", |(_, n)| n);
    let body = col().w_full().px(24.0).pt(6.0).pb(20.0)
        .child(row().w_full().pt(10.0).pb(4.0).child(text(title).font(BODY_BOLD).color(p.text)))
        .child(pane(s, p));
    let content = col().key("settings-pane").grow(1.0).min_w(0.0).h_full()
        .scroll(ScrollSpec { stick_bottom: false }).child(body);
    let bar = row().w_full().h(44.0).px(16.0).center_y()
        .child(text("Settings").font(BODY_BOLD).color(p.text).grow(1.0))
        .child(button("Done", p.on_selection, p.accent, p).default_button().on_press(|s: &mut App| s.close_sheet()));
    let card = col().key("settings").w(780.0).h(580.0).radius(12.0).clip_rounded(12.0)
        .fill(p.content).border(1.0, p.separator).shadow(16.0, 48.0, Color::BLACK.alpha(0.3))
        .modal(true, |s: &mut App| s.close_sheet())
        .child(bar)
        .child(el().w_full().h(1.0).fill(p.separator))
        .child(row().w_full().grow(1.0).min_h(0.0)
            .child(nav)
            .child(el().w(1.0).h_full().fill(p.separator))
            .child(content));
    el().key("settings-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(p.scrim).center().child(card)
}

fn note(words: &str, p: Palette) -> El {
    row().w_full().py(6.0).child(text(words.to_owned()).font(SMALL).color(p.tertiary).wrap().grow(1.0).min_w(0.0))
}

fn pane(s: &App, p: Palette) -> El {
    let st = &s.settings;
    let c = col().w_full();
    match s.settings_pane {
        SettingsPane::General => c
            .child(setting("Check for new mail", None,
                segmented("set-refresh", &[("1 min", 1), ("5 min", 5), ("15 min", 15), ("Manually", 0)],
                    st.refresh_minutes, |st, v| st.refresh_minutes = v, p), p))
            .child(setting("Swipe all the way left to", Some("Two fingers sideways on a message. Right toggles read."),
                segmented("set-swipe", &[("Archive", SwipeLeft::Archive), ("Delete", SwipeLeft::Delete)],
                    st.swipe_left, |st, v| st.swipe_left = v, p), p))
            .child(setting("Undo send", Some("How long a sent message waits, so Undo can bring it back."),
                segmented("set-undo-send", &[("Off", 0), ("5 s", 5), ("10 s", 10), ("20 s", 20)],
                    st.undo_send_secs, |st, v| st.undo_send_secs = v, p), p))
            .child(setting("Mark messages as read when opened", None,
                switch("set-read", st.mark_read_on_open, |st, v| st.mark_read_on_open = v, p), p)),
        SettingsPane::Viewing => c
            .child(setting("Appearance", None,
                segmented("set-appearance", &[("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)],
                    st.appearance, |st, v| st.appearance = v, p), p))
            .child(setting("Organize by conversation", Some("One row per thread; the reader shows the whole conversation."),
                switch("set-threads", st.conversations, |st, v| st.conversations = v, p), p))
            .child(setting("Newest message at the top", None,
                switch("set-newest", st.newest_first, |st, v| st.newest_first = v, p), p))
            .child(setting("List preview", None,
                segmented("set-preview", &[("None", 0), ("1 line", 1), ("2 lines", 2), ("3 lines", 3)],
                    st.preview_lines, |st, v| st.preview_lines = v, p), p))
            .child(setting("Show HTML email", Some("Off shows every message as plain text."),
                switch("set-html", st.render_html, |st, v| st.render_html = v, p), p)),
        SettingsPane::Accounts => c
            .child(note("The label is what the sidebar calls an account; only you see it. The name is what people \
                see on mail you send.", p))
            .children(s.accounts.iter().enumerate().map(|(i, a)| account_row(s, a, i, p)))
            .child(row().w_full().pt(8.0)
                .child(button("Add Account…", p.text, p.field, p).on_press(|s: &mut App| s.add_account()))),
        SettingsPane::Privacy => c
            .child(setting("Load remote content", Some("Remote images can tell senders when and where you read a message. Off, you can load them per message."),
                switch("set-remote", st.remote_images, |st, v| st.remote_images = v, p), p))
            .child(note("Trackers are always blocked: spy pixels and known open-tracking services never load, \
                even with remote content on. Links lose tracking parameters and redirect wrappers when you open them. \
                Read receipts are never sent, and sent mail and image requests don't carry this computer's name.", p)),
        SettingsPane::Storage => storage(s, p),
        SettingsPane::Ai => c.gap(6.0)
            .child(text("Ctrl+Shift+C (or y) copies the open conversation, or the marked ones, as a prompt for your AI tool. \
                It starts with this:").font(SMALL).color(p.secondary).wrap())
            .child(row().w_full().h(120.0).px(10.0).py(6.0).radius(7.0).fill(p.field)
                .child(super::field::input(&s.ai_prompt_edit, BODY, p).key("ai-prompt").grow(1.0).h_full()
                    .on_edit(|s: &mut App| &mut s.ai_prompt_edit)
                    .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| {
                        if e.key == rune::Key::Enter && !e.mods.cmd {
                            s.ai_prompt_edit.insert("\n");
                            return true;
                        }
                        super::field::platform_key(&mut s.ai_prompt_edit, e)
                    })
                    .on_input_change(|s: &mut App, tx| s.edit_ai_prompt(tx)))),
        SettingsPane::Keyboard => c
            .child(setting("Vim keys", Some("j/k to move, x to mark, d to delete, and more. Press ? to see them all."),
                switch("set-vim", st.vim, |st, v| st.vim = v, p), p)),
    }
}

/// Bytes as people say them: "312 MB", "1.4 GB".
pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    match mb {
        m if m >= 1024.0 => format!("{:.1} GB", m / 1024.0).replace(".0 ", " "),
        m if m >= 10.0 => format!("{m:.0} MB"),
        m if m >= 1.0 => format!("{m:.1} MB"),
        _ => format!("{} KB", bytes.div_ceil(1024)),
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn storage(s: &App, p: Palette) -> El {
    let st = &s.settings.storage;
    let on = st.on;
    // Turning it off deletes what's kept, so that press asks first, saying how much.
    let toggle = switch_for("set-storage", on, p).on_press_tx(move |s: &mut App, tx| {
        if on {
            s.confirm_storage_off = true;
        } else {
            s.update_settings(|st| st.storage.on = true, tx);
        }
    });
    let mut c = col().w_full()
        .child(setting("Keep mail on this computer", Some("The newest mail of every account, its attachments \
            included, so it opens at once, reads offline and searches fast. Downloaded in the background; \
            nothing is marked read. Encrypted, with the key in your system keyring."), toggle, p));
    if s.confirm_storage_off {
        let kept = s.storage_usage.as_ref().map_or_else(|| "everything kept".to_owned(), |(u, _)| size(u.bytes));
        c = c.child(row().w_full().py(8.0).gap(8.0).center_y()
            .child(text(format!("Turning it off deletes {kept} from this computer.")).font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).wrap())
            .child(button("Cancel", p.text, p.field, p).on_press(|s: &mut App| s.confirm_storage_off = false))
            .child(button(&format!("Delete {kept}"), p.on_selection, rune::color::hex("#ff3b30").unwrap(), p).key("storage-delete")
                .on_press_tx(|s: &mut App, tx| {
                    s.confirm_storage_off = false;
                    s.update_settings(|st| st.storage.on = false, tx);
                })));
    }
    if !on {
        return c.child(note("Off, mail is only on the server: each message downloads when you open it, and \
            nothing is written here (actions made offline wait in memory until the network's back).", p));
    }
    let budget = st.budget_bytes();
    let (meter, line) = match &s.storage_usage {
        Some((u, _)) => {
            let of = budget.map_or_else(|| "no limit".to_owned(), |b| format!("of {}", size(b)));
            let mut line = format!("{} {of} · {} messages", size(u.bytes), thousands(u.messages));
            if let Some(d) = u.oldest.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()) {
                line.push_str(&format!(" · back to {}", d.format("%-d %B %Y")));
            }
            (budget.map(|b| (u.bytes as f64 / b as f64).clamp(0.0, 1.0)), line)
        }
        None if s.store.is_some() => (None, "Measuring…".to_owned()),
        None => (None, "Nothing kept yet".to_owned()),
    };
    let bar = el().key("storage-meter").w_full().h(8.0).radius(4.0).fill(p.field).clip_rounded(4.0)
        .child(el().h_full().w_frac(meter.unwrap_or(0.0) as f32).radius(4.0).fill(p.accent));
    c.child(setting("Use up to", Some("The oldest goes first when it's full; what you opened this week stays."),
            segmented("set-budget", &[("250 MB", 250), ("1 GB", 1024), ("5 GB", 5120), ("10 GB", 10240), ("No limit", 0)],
                st.budget_mb, |st, v| st.storage.budget_mb = v, p), p))
        .child(col().w_full().py(8.0).gap(6.0)
            .child(bar)
            .child(row().w_full().gap(8.0).center_y()
                .child(text(line).key("storage-usage").font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).truncate())
                .child(button("Clear", p.text, p.field, p).key("storage-clear").help("Delete what's kept; it downloads again")
                    .on_press(|s: &mut App| s.clear_storage()))))
        .child(setting("Keep remote images", Some("Only for messages you loaded them in; never downloaded ahead. \
            Trackers stay blocked."),
            switch("set-keep-remote", st.remote_images, |st, v| st.storage.remote_images = v, p), p))
        .child(setting("Download on metered connections", Some("Off, downloading ahead waits while you're on a phone's hotspot."),
            switch("set-metered", st.metered, |st, v| st.storage.metered = v, p), p))
}

fn account_row(s: &App, a: &crate::app::Account, index: usize, p: Palette) -> El {
    let (key, kind, calendar) = (a.key.as_str(), a.kind, a.calendar);
    let k = key.to_owned();
    let confirming = s.confirm_remove.as_deref() == Some(key);
    let remove = if confirming {
        button("Remove Account", p.on_selection, rune::color::hex("#ff3b30").unwrap(), p)
            .on_press(move |s: &mut App| s.remove_account(&k))
    } else {
        button("Remove", p.text, p.field, p).on_press(move |s: &mut App| s.confirm_remove = Some(k.clone()))
    };
    let k = key.to_owned();
    // An IMAP account's own app password opens its calendar, so the switch just shows or
    // hides it; a Fastmail JMAP account asks for an app password when switched on.
    let own = a.own_calendar;
    let cal_button = row().gap(6.0).center_y()
        .child(text("Calendar").font(SMALL).color(p.secondary))
        .child(switch_for(&format!("cal-{key}"), calendar, p).on_press(move |s: &mut App| match (own, calendar) {
            (true, on) => s.set_calendar_enabled(&k, !on),
            (false, true) => s.remove_calendar(&k),
            (false, false) => s.edit_calendar(&k),
        }));
    let detail = match (confirming, kind, calendar) {
        (true, Kind::Fastmail, _) => "Removing deletes the saved token for this account.".to_owned(),
        (true, _, _) => "Removing deletes the saved password for this account.".to_owned(),
        (false, Kind::Fastmail, _) => "Fastmail".to_owned(),
        (false, Kind::Gmail, _) => format!("Gmail · {key}"),
        (false, Kind::Imap, _) if a.own_calendar => format!("Fastmail over IMAP · {key}"),
        (false, Kind::Imap, _) => format!("IMAP and SMTP · {key}"),
    };
    let detail = match (confirming, kind) {
        (false, Kind::Fastmail) => format!("{detail} · {key}"),
        _ => detail,
    };
    let (kl, ku, kd) = (key.to_owned(), key.to_owned(), key.to_owned());
    let kb = key.to_owned();
    let label_input = super::field::input(&a.label_edit, BODY, p).key(format!("label-{key}")).single_line().grow(1.0)
            .placeholder(key, p.tertiary)
            .on_edit(move |s: &mut App| s.label_field(&kl))
            .on_input_key({
                let k = key.to_owned();
                move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(s.label_field(&k), e)
            })
            .on_input_change({
                let k = key.to_owned();
                move |s: &mut App, _| s.save_label(&k)
            });
    let label = super::field::boxed(label_input, 28.0, 8.0, move |s| s.label_field(&kb), p).grow(1.0);
    let (kn, kb) = (key.to_owned(), key.to_owned());
    let name_input = super::field::input(&a.from_name_edit, BODY, p).key(format!("from-name-{key}")).single_line().grow(1.0)
            .placeholder(if kind == Kind::Fastmail { "Your Fastmail identity's name" } else { "Your name" }, p.tertiary)
            .on_edit(move |s: &mut App| s.sender_name_field(&kn))
            .on_input_key({
                let k = key.to_owned();
                move |s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(s.sender_name_field(&k), e)
            })
            .on_input_change({
                let k = key.to_owned();
                move |s: &mut App, _| s.save_from_name(&k)
            });
    let name = super::field::boxed(name_input, 28.0, 8.0, move |s| s.sender_name_field(&kb), p).grow(1.0);
    let labelled = |what: &str, field: El| col().grow(1.0).min_w(0.0).gap(3.0)
        .child(text(what.to_owned()).font(SMALL).color(p.secondary))
        .child(field);
    let last = index + 1 == s.accounts.len();
    let order = row().gap(2.0)
        .child(arrow(&style::CHEVRON_UP, "Move up", index > 0, p).on_press(move |s: &mut App| s.move_account(&ku, -1)))
        .child(arrow(&style::CHEVRON_DOWN, "Move down", !last, p).on_press(move |s: &mut App| s.move_account(&kd, 1)));
    let mut block = col().w_full().py(8.0).gap(8.0)
        .child(row().w_full().gap(8.0).center_y()
            .child(order)
            .child(col().grow(1.0).min_w(0.0).gap(6.0)
                .child(text(detail).font(SMALL).color(p.secondary).truncate())
                .child(row().w_full().gap(8.0)
                    .child(labelled("Label in the sidebar", label))
                    .child(labelled("Name on sent mail", name))))
            .child(if kind == Kind::Fastmail || a.own_calendar { cal_button } else { el() })
            .child(remove));
    if s.calendar_edit.as_deref() == Some(key) {
        let input = super::field::boxed(super::field::input(&s.calendar_field, BODY, p).key("calendar-password").single_line().secure().grow(1.0)
                .on_input_key(|s: &mut App, e: &rune::KeyEvent, _: &mut rune::Tx| super::field::platform_key(&mut s.calendar_field, e)).placeholder("App password", p.tertiary)
                .on_edit(|s: &mut App| &mut s.calendar_field)
                .on_submit(|s: &mut App, _| s.save_calendar()), 30.0, 10.0, |s| &mut s.calendar_field, p);
        block = block
            .child(text("Fastmail calendars need an app password: in Fastmail, open Settings → Privacy & Security → \
                         Manage app passwords, and create one with access to Calendars (CalDAV). It's saved with \
                         your accounts, readable only by you.")
                .font(SMALL).color(p.secondary).wrap())
            .child(row().w_full().gap(8.0).center_y()
                .child(input.grow(1.0))
                .child(button("Cancel", p.text, p.field, p).on_press(|s: &mut App| s.calendar_edit = None))
                .child(button("Connect", p.on_selection, p.accent, p).on_press(|s: &mut App| s.save_calendar())));
    }
    block
}

fn arrow(i: &'static rune::Icon, help: &str, enabled: bool, p: Palette) -> El {
    el().size(22.0, 22.0).radius(5.0).center().hover(p.hover).help(help).opacity(if enabled { 1.0 } else { 0.3 })
        .child(rune::icon(i, 12.0).color(p.secondary))
}

/// A switch drawn like `switch`, whose press the caller handles.
fn switch_for(key: &str, on: bool, p: Palette) -> El {
    let knob = el().size(16.0, 16.0).radius(8.0).fill(Color::WHITE).shadow(1.0, 2.0, Color::BLACK.alpha(0.25))
        .absolute(if on { 16.0 } else { 2.0 }, 2.0).motion(rune::tok::motion::SNAPPY);
    el().key(key.to_owned()).role("switch").focusable(true).size(34.0, 20.0).radius(10.0)
        .fill(if on { p.accent } else { p.field.alpha(0.25).mix(p.secondary, 0.5) })
        .motion(rune::tok::motion::SNAPPY)
        .child(knob)
}

pub fn shortcuts(s: &App, p: Palette) -> El {
    let entry = |keys: &str, what: &str| {
        row().w_full().py(3.0).gap(12.0)
            .child(text(keys.to_owned()).font(SMALL.mono()).color(p.text).w(150.0))
            .child(text(what.to_owned()).font(SMALL).color(p.secondary).grow(1.0).min_w(0.0).wrap())
    };
    let mut left = col().grow(1.0).min_w(0.0);
    for (title, list) in SHORTCUTS {
        left = left.child(section(title, p)).children(list.iter().map(|(k, w)| entry(k, w)));
    }
    let mut columns = row().w_full().gap(28.0).items_start().child(left);
    if s.settings.vim {
        let right = col().grow(1.0).min_w(0.0).child(section("Vim keys", p))
            .children(VIM.iter().map(|(k, w)| entry(k, w)));
        columns = columns.child(right);
    }
    let note = text(if cfg!(target_os = "macos") { "Ctrl means ⌘ on this Mac." } else {
        "Vim keys work while no text field has focus. Turn them off in Settings."
    }).font(SMALL).color(p.tertiary).wrap();
    sheet("shortcuts", "Keyboard Shortcuts", if s.settings.vim { 820.0 } else { 460.0 },
        col().w_full().px(20.0).pb(20.0).child(columns).child(row().pt(14.0).child(note)), p)
}
