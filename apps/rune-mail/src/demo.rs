//! Sample mail for headless snapshots: `cargo run -- --snap mail.png --scene demo`.

use chrono::{Local, TimeZone};

use crate::app::{Account, App, Phase};
use crate::backend::Kind;
use crate::caldav::Event;
use crate::jmap::{Addr, Body, Mailbox, Summary};
use crate::worker::View;

const HOME: &str = "ted@fastmail.com";
const WORK: &str = "ted.martin@gmail.com";

fn uid(id: &str) -> String {
    let account = if id.starts_with('w') { WORK } else { HOME };
    crate::jmap::uid(account, id)
}

fn mailbox(id: &str, name: &str, role: Option<&str>, parent: Option<&str>, unread: u32, total: u32) -> Mailbox {
    Mailbox {
        id: id.into(),
        name: name.into(),
        role: role.map(Into::into),
        parent_id: parent.map(Into::into),
        sort_order: 0,
        unread,
        total,
    }
}

fn standard(prefix: &str, unread: u32) -> Vec<Mailbox> {
    let id = |s: &str| format!("{prefix}{s}");
    vec![
        mailbox(&id("in"), "Inbox", Some("inbox"), None, unread, 1284),
        mailbox(&id("dr"), "Drafts", Some("drafts"), None, 0, 2),
        mailbox(&id("se"), "Sent", Some("sent"), None, 0, 512),
        mailbox(&id("ar"), "Archive", Some("archive"), None, 0, 9120),
        mailbox(&id("ju"), "Junk", Some("junk"), None, 4, 4),
        mailbox(&id("tr"), "Trash", Some("trash"), None, 0, 31),
    ]
}

/// One account, its inbox shown.
pub fn single(s: &mut App) {
    s.offline();
    s.phase = Phase::Ready;
    let mut home = standard("", 3);
    home.extend([
        mailbox("f-work", "Work", None, None, 1, 88),
        mailbox("f-inv", "Invoices", None, Some("f-work"), 0, 40),
        mailbox("f-rec", "Receipts", None, None, 0, 120),
    ]);
    s.accounts = vec![Account {
        key: HOME.into(),
        kind: Kind::Fastmail,
        label: Some("Personal".into()),
        label_edit: rune::TextEdit::new("Personal"),
        from_name_edit: rune::TextEdit::new("Ada Lovelace"),
        position: 0,
        own_calendar: false,
        live: true,
        mailboxes: home,
        calendar: true,
        events: events(),
        contacts: vec![
            crate::contacts::Contact { name: "Charles Babbage".into(), emails: vec!["charles@example.com".into(), "cb@engine.example".into()] },
            crate::contacts::Contact { name: "Charlotte Brontë".into(), emails: vec!["charlotte@haworth.example".into()] },
            crate::contacts::Contact { name: "Richard Feynman".into(), emails: vec!["feynman@caltech.example".into()] },
        ],
    }];
    s.view = Some(View::Mailbox { account: HOME.into(), id: "in".into() });
    messages(s, false);
}

/// Two accounts in All Inboxes, with today's events.
pub fn scene(s: &mut App) {
    single(s);
    let mut work = standard("w", 2);
    work.push(mailbox("wf-proj", "Projects", None, None, 0, 14));
    let standup = Event { title: "Team standup".into(), location: Some("Zoom".into()), calendar: "Work".into(),
        color: Some("#34c759".into()), ..at(9, 0, 15) };
    s.accounts.push(Account {
        key: WORK.into(),
        kind: Kind::Gmail,
        label: None,
        label_edit: rune::TextEdit::new(""),
        from_name_edit: rune::TextEdit::new(""),
        position: 1,
        own_calendar: true,
        live: true,
        mailboxes: work,
        calendar: true,
        events: Vec::new(),
        contacts: Vec::new(),
    });
    s.accounts[0].events.insert(1, standup);
    s.view = Some(View::AllInboxes);
    messages(s, true);
}

fn at(hour: u32, minute: u32, minutes: i64) -> Event {
    let start = Local::now().date_naive().and_hms_opt(hour, minute, 0).and_then(|t| Local.from_local_datetime(&t).earliest());
    Event {
        title: String::new(),
        location: None,
        start,
        end: start.map(|s| s + chrono::Duration::minutes(minutes)),
        calendar: "Personal".into(),
        color: Some("#0a84ff".into()),
    }
}

fn events() -> Vec<Event> {
    vec![
        Event { title: "Mum's birthday".into(), start: None, end: None, color: Some("#ff9500".into()), ..at(0, 0, 0) },
        Event { title: "Lunch with Grace".into(), location: Some("Café Hopper".into()), ..at(12, 30, 60) },
        Event { title: "Dentist".into(), ..at(16, 0, 45) },
    ]
}

fn messages(s: &mut App, with_work: bool) {
    let msg = |id: &str, name: &str, email: &str, subject: &str, preview: &str, at: &str, seen: bool, flagged: bool| {
        let account = if id.starts_with('w') { WORK } else { HOME };
        Summary {
            id: id.into(),
            account: account.into(),
            uid: uid(id),
            thread_id: id.into(),
            thread_key: uid(id),
            from: vec![Addr { name: Some(name.into()), email: email.into() }],
            to: vec![Addr { name: Some("Ted Martin".into()), email: account.into() }],
            subject: subject.into(),
            preview: preview.into(),
            received_at: at.into(),
            seen,
            flagged,
            has_attachment: id == "m2",
            mailbox_ids: vec![if id.starts_with('w') { "win".into() } else { "in".into() }],
        }
    };
    let now = chrono::Utc::now();
    let ago = |h: i64| (now - chrono::Duration::hours(h)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut list = vec![
        msg("m1", "Ada Lovelace", "ada@example.com", "Notes on the analytical engine",
            "I've finished the translation and added a few notes of my own. Note G turned out longer than expected.",
            &ago(1), false, true),
        msg("m2", "Fastmail", "support@fastmail.com", "Your invoice is ready",
            "Thanks for being a Fastmail customer. Your invoice for this month is attached.", &ago(3), false, false),
        msg("m3", "Grace Hopper", "grace@example.com", "Re: Compiler meeting",
            "Sounds good. Let's meet Thursday at 2pm and bring the moth.", &ago(30), true, false),
        msg("m4", "Linus", "linus@example.com", "Patch review",
            "The patch looks fine but the commit message needs work. Please explain why, not what.", &ago(80), false, false),
        msg("m5", "Margaret Hamilton", "margaret@example.com", "Priority displays",
            "The 1202 alarm is expected under load. The executive sheds low-priority jobs.", &ago(400), true, false),
    ];
    if with_work {
        list.push(msg("w1", "Katherine Johnson", "katherine@work.example", "Trajectory check",
            "I ran the numbers by hand. The launch window holds if we go before 9:40.", &ago(2), false, false));
        list.push(msg("w2", "Dorothy Vaughan", "dorothy@work.example", "FORTRAN training",
            "The new IBM arrives Monday. I've booked the team for the training sessions.", &ago(50), true, false));
        list.sort_by(|a, b| b.received_at.cmp(&a.received_at));
    }
    s.threads = list.iter().map(|m| (m.thread_key.clone(), vec![m.clone()])).collect();
    // Grace's message is the newest of a conversation that includes Ted's reply in Sent.
    let mut earlier = msg("m3a", "Grace Hopper", "grace@example.com", "Compiler meeting",
        "Can we meet this week about the compiler? I have a bug to show you. It's in the relay.", &ago(54), true, false);
    let mut reply = msg("m3b", "Ted Martin", HOME, "Re: Compiler meeting",
        "Sure, Thursday works. What time suits you?", &ago(41), true, false);
    reply.to = vec![Addr { name: Some("Grace Hopper".into()), email: "grace@example.com".into() }];
    reply.mailbox_ids = vec!["se".into()];
    let mut first = msg("m3x", "Grace Hopper", "grace@example.com", "Compiler meeting",
        "I've been thinking about the A-0 system and whether we can make programs read more like English.", &ago(78), true, false);
    let mut ted_first = msg("m3y", "Ted Martin", HOME, "Re: Compiler meeting",
        "That's a big idea. Most people here think computers can only do arithmetic.", &ago(66), true, true);
    ted_first.mailbox_ids = vec!["se".into()];
    let latest = list.iter().find(|m| m.id == "m3").cloned().unwrap();
    for m in [&mut first, &mut ted_first, &mut earlier, &mut reply] {
        m.thread_id = "m3".into();
        m.thread_key = latest.thread_key.clone();
    }
    s.threads.insert(latest.thread_key.clone(), vec![first, ted_first, earlier, reply, latest]);
    s.messages = list;
    s.selected = Some(uid("m1"));
    s.bodies.insert(uid("m1"), Body { html_src: None, inline: None,
        files: vec![
            crate::jmap::FileInfo { name: "Sketch of the Analytical Engine.pdf".into(), mime: "application/pdf".into(), size: 2_480_000, part: "b1".into() },
            crate::jmap::FileInfo { name: "note-g-table.png".into(), mime: "image/png".into(), size: 184_000, part: "b2".into() },
        ],
        text: "Hi Ted,\n\nI've finished the translation of Menabrea's paper and added a few notes of my own. \
               Note G turned out longer than expected: it walks through computing the Bernoulli numbers \
               on the engine, step by step.\n\nI'd love your thoughts before it goes to Taylor's.\n\nBest,\nAda"
            .into(),
        html: None,
        cc: vec![Addr { name: Some("Charles Babbage".into()), email: "charles@example.com".into() }],
        reply_to: vec![],
        message_id: vec![],
        references: vec![],
        unsubscribe: None,
    });
}

const INVOICE_HTML: &str = r##"<html><body style="margin:0;background:#f3f4f6"><div style="display:none">Preheader text nobody should see</div>
<table width="600" align="center" bgcolor="#ffffff" cellpadding="0" style="margin:0 auto">
<tr><td bgcolor="#0067b9" style="padding:20px;color:#ffffff;text-align:center"><img src="data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48cmVjdCB3aWR0aD0iOTYiIGhlaWdodD0iOTYiIHJ4PSIyMCIgZmlsbD0iIzAwNjdiOSIvPjxwYXRoIGQ9Ik0yNCAzNGg0OHYzMEgyNHoiIGZpbGw9Im5vbmUiIHN0cm9rZT0iI2ZmZiIgc3Ryb2tlLXdpZHRoPSI1Ii8+PHBhdGggZD0iTTI0IDM0bDI0IDE4IDI0LTE4IiBmaWxsPSJub25lIiBzdHJva2U9IiNmZmYiIHN0cm9rZS13aWR0aD0iNSIvPjwvc3ZnPg==" width="48" height="48"><h1 style="color:#ffffff">Your invoice is ready</h1></td></tr>
<tr><td style="padding:24px;color:#333333;font-size:15px">
<p>Hi Ted,</p>
<p>Thanks for being a <b>Fastmail</b> customer. Your invoice for <i>October 2026</i> is attached. You can also <a href="https://www.fastmail.com/settings/billing">view it online</a>.</p>
<table width="100%" cellpadding="8"><tr><td width="70%" bgcolor="#f3f4f6"><b>Standard plan</b></td><td width="30%" bgcolor="#f3f4f6" align="right">$5.00</td></tr>
<tr><td width="70%">Tax</td><td width="30%" align="right">$0.50</td></tr>
<tr><td width="70%"><b>Total</b></td><td width="30%" align="right"><b>$5.50</b></td></tr></table>
<ul><li>Payment is taken on the 1st.</li><li>Questions? Write to <a href="mailto:billing@fastmail.com">billing@fastmail.com</a>.</li></ul>
<blockquote>We never sell your data.</blockquote>
<hr><p style="font-size:12px;color:#888888;text-align:center">Fastmail Pty Ltd · Melbourne, Australia</p>
<img src="https://tracker.example/open.gif" width="1" height="1"><img src="https://acme.us1.list-manage.com/track/open.php?u=1&id=2"><a href="https://acme.us1.list-manage.com/track/click?u=1">Manage preferences</a><img src="https://cdn.example/banner.png" width="600" height="80">
</td></tr></table></body></html>"##;

pub fn html_scene(s: &mut App) {
    scene(s);
    let body = Body { html_src: Some(INVOICE_HTML.into()), inline: None, files: Vec::new(),
        text: "Your invoice is ready".into(),
        html: Some(crate::html::parse(INVOICE_HTML, &|_| None)),
        cc: vec![],
        reply_to: vec![],
        message_id: vec![],
        references: vec![],
        unsubscribe: crate::privacy::Unsubscribe::parse(
            "<mailto:unsubscribe@fastmail.example>, <https://www.fastmail.example/unsubscribe/abc>",
            Some("List-Unsubscribe=One-Click"),
            Some("Fastmail Billing <billing.fastmail.example>"),
        ),
    };
    s.bodies.insert(uid("m2"), body);
    s.selected = Some(uid("m2"));
    s.request_images(&uid("m2"));
}

pub fn marked_scene(s: &mut App) {
    scene(s);
    s.marked.extend([uid("m2"), uid("m3"), uid("w1")]);
}

pub fn settings_scene(s: &mut App) {
    scene(s);
    s.sheet = crate::app::Sheet::Settings;
}

pub fn shortcuts_scene(s: &mut App) {
    scene(s);
    s.sheet = crate::app::Sheet::Shortcuts;
}

pub fn compose_scene(s: &mut App) {
    scene(s);
    s.reply(true);
}

pub fn calendar_scene(s: &mut App) {
    settings_scene(s);
    s.accounts[1].calendar = false;
    s.edit_calendar(WORK);
}

pub fn add_account_scene(s: &mut App) {
    scene(s);
    s.add_account();
    s.choose_add_kind(crate::app::AddKind::Other);
}

/// The three-message conversation open, its newest message expanded.
pub fn thread_scene(s: &mut App) {
    scene(s);
    for (id, text) in [
        ("m3x", "Ted,\n\nI've been thinking about the A-0 system and whether we can make programs read more like English. \
                 If a compiler can translate symbols into machine code, it can translate words too. Business people \
                 would rather write PAY = HOURS * RATE than octal.\n\nThe hard part isn't the machine, it's convincing \
                 people it can be done. I'd like to show the team a working version by spring, and I'd value your help \
                 with the subroutine library.\n\nGrace"),
        ("m3y", "That's a big idea. Most people here think computers can only do arithmetic. Count me in.\n\nTed"),
        ("m3a", "Can we meet this week about the compiler? I have a bug to show you.\n\nIt's in the relay.\n\nGrace"),
        ("m3b", "Sure, Thursday works. What time suits you?\n\nTed"),
        ("m3", "Sounds good. Let's meet Thursday at 2pm and bring the moth.\n\nGrace"),
    ] {
        s.bodies.insert(uid(id), Body { html_src: None, inline: None, files: Vec::new(),
            text: text.into(),
            html: None,
            cc: vec![],
            reply_to: vec![],
            message_id: vec![],
            references: vec![],
            unsubscribe: None,
        });
    }
    s.select_message(uid("m3"));
}

/// The conversation, its newest message on a white page (for dark snapshots).
pub fn white_scene(s: &mut App) {
    thread_scene(s);
    s.on_white.insert(uid("m3"));
}

/// Held by tests that start the app: they share the scratch settings and store, and one
/// launching with storage off would empty what another just kept.
#[cfg(test)]
pub fn test_app() -> std::sync::MutexGuard<'static, ()> {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    test_config();
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Points config at an empty scratch folder, so tests never read or migrate real accounts.
#[cfg(test)]
pub fn test_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("rune-mail-test-{}", std::process::id()));
        // SAFETY: set once, before any test thread reads the environment through `dirs`.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            std::env::set_var("XDG_DATA_HOME", &dir);
            std::env::set_var("RUNE_MAIL_NO_KEYRING", "1");
            std::env::remove_var("FASTMAIL_TOKEN");
        }
    });
}

/// A new message with "ch" typed in To: suggestions from contacts and mail.
pub fn suggest_scene(s: &mut App) {
    scene(s);
    s.compose_new();
    s.compose.to = rune::TextEdit::new("ch");
    s.compose.to.focused = true;
}

/// The open conversation's newest message failed to load.
pub fn failed_scene(s: &mut App) {
    thread_scene(s);
    s.bodies.remove(&uid("m3"));
    s.body_failed.insert(uid("m3"), "imap.example.com: connection reset by peer".into());
}

/// A new message, To focused, nothing typed.
pub fn new_message_scene(s: &mut App) {
    scene(s);
    s.compose_new();
}
