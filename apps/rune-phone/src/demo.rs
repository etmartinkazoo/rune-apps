//! Headless scenes (`--scene NAME`): a made-up phone, no Bluetooth.

use crate::app::{App, Pane, Tab};
use crate::bluez::Device;
use crate::calls::{LiveCall, State};
use crate::vcard::{Call, CallKind, Contact};
use crate::worker::{Link, Message};

fn today(hhmm: &str) -> String {
    format!("{}T{hhmm}00", chrono::Local::now().format("%Y%m%d"))
}

fn days_ago(n: i64, hhmm: &str) -> String {
    format!("{}T{hhmm}00", (chrono::Local::now() - chrono::Duration::days(n)).format("%Y%m%d"))
}

fn msg(id: u32, number: &str, text: &str, at: String, incoming: bool) -> Message {
    Message { id: format!("m{id}"), number: number.into(), name: String::new(), text: text.into(), at, incoming, read: true, mms: false, partial: false }
}

fn phone() -> Device {
    Device { path: "/org/bluez/hci0/dev_AA".into(), address: "AA:BB:CC:DD:EE:FF".into(), name: "Pixel 9".into(), paired: true, connected: true, phone: true }
}

pub fn ready(s: &mut App) {
    s.demo = true;
    // The scene's own phone and pins, not this computer's.
    s.settings = crate::settings::Settings::default();
    s.settings.phone = Some("AA:BB:CC:DD:EE:FF".into());
    s.link = Link { phone: Some(phone()), calls: true, messages: true, contacts: true };
    s.devices = vec![phone()];
    let contacts = vec![
        Contact { name: "Ada Lovelace".into(), numbers: vec![("mobile".into(), "+15550100100".into())] },
        Contact { name: "Grace Hopper".into(), numbers: vec![("mobile".into(), "+15550100200".into()), ("work".into(), "+15550100201".into())] },
        Contact { name: "Katherine Johnson".into(), numbers: vec![("mobile".into(), "+15550100300".into())] },
        Contact { name: "Margaret Hamilton".into(), numbers: vec![("home".into(), "+15550100400".into())] },
    ];
    s.take_contacts(contacts);
    let mut unread = msg(9, "+15550100300", "Launch window moved to 9:40. Can you double-check the numbers?", today("0912"), true);
    unread.read = false;
    s.take_messages(vec![
        msg(1, "+15550100100", "Did you get a chance to read Note G?", days_ago(1, "1820"), true),
        msg(2, "+15550100100", "Yes! The Bernoulli numbers part is brilliant", days_ago(1, "1832"), false),
        msg(3, "+15550100100", "Thank you. Babbage thinks the engine could do it in an afternoon", today("0805"), true),
        msg(4, "+15550100100", "Lunch Thursday to talk it through?", today("0806"), true),
        msg(5, "+15550100100", "Thursday works. Noon at the usual place", today("0811"), false),
        msg(6, "+15550100200", "Found the moth. It was in relay 70", days_ago(3, "1545"), true),
        msg(7, "+15550100200", "Taping it into the logbook 🐛", days_ago(3, "1547"), false),
        unread,
        msg(10, "+15550199999", "Your code is 482913. It expires in 10 minutes.", days_ago(2, "0730"), true),
    ]);
    s.recents = vec![
        Call { name: "Ada Lovelace".into(), number: "+15550100100".into(), kind: CallKind::Outgoing, at: today("0830") },
        Call { name: "Katherine Johnson".into(), number: "+15550100300".into(), kind: CallKind::Missed, at: today("0705") },
        Call { name: String::new(), number: "+15550177777".into(), kind: CallKind::Incoming, at: days_ago(1, "1610") },
    ];
    let key = crate::number::key("+15550100100");
    s.open_thread(&key);
}

pub fn incoming(s: &mut App) {
    ready(s);
    s.calls = vec![LiveCall { path: "/c0".into(), number: "+15550100200".into(), name: String::new(), state: State::Incoming }];
}

/// Five conversations pinned: a full row of three and two more, over the rest.
pub fn pinned(s: &mut App) {
    ready(s);
    let mut more = s.messages.clone();
    for (i, (name, number, text)) in [
        ("Dorothy Vaughan", "+15550100500", "FORTRAN class is at 2"),
        ("Mary Jackson", "+15550100600", "Wind tunnel results are in"),
        ("Hedy Lamarr", "+15550100700", "Frequency hopping, remember?"),
        ("Radia Perlman", "+15550100800", "Spanning tree poem attached"),
    ]
    .into_iter()
    .enumerate()
    {
        s.contacts.push(Contact { name: name.into(), numbers: vec![("mobile".into(), number.into())] });
        more.push(msg(20 + i as u32, number, text, days_ago(i as i64 + 1, "1200"), true));
    }
    let contacts = s.contacts.clone();
    s.take_contacts(contacts);
    s.take_messages(more);
    for n in ["+15550100100", "+15550100200", "+15550100300", "+15550100500", "+15550100600"] {
        s.settings.pinned.push(crate::number::key(n));
    }
}

pub fn on_call(s: &mut App) {
    ready(s);
    s.calls = vec![LiveCall { path: "/c0".into(), number: "+15550100100".into(), name: String::new(), state: State::Active }];
    s.call_started = Some(-83.0);
    s.call_keypad = true;
}

pub fn keypad(s: &mut App) {
    ready(s);
    s.tab = Tab::Calls;
    s.pane = Pane::Keypad;
    s.dial = rune::TextEdit::new("5550100300");
}

pub fn contact(s: &mut App) {
    ready(s);
    s.tab = Tab::Contacts;
    s.pane = Pane::Contact(1);
}

pub fn setup(s: &mut App) {
    s.demo = true;
    s.devices = vec![
        phone(),
        Device { path: "/n1".into(), address: "11:22:33:44:55:66".into(), name: "Galaxy S24".into(), paired: false, connected: false, phone: true },
    ];
    s.discovering = true;
}

pub fn pair(s: &mut App) {
    setup(s);
    s.pair_code = Some(("Pixel 9".into(), 48_213));
}

/// Asking whether to remove the phone in use.
pub fn remove(s: &mut App) {
    ready(s);
    s.setup_open = true;
    s.removing = Some(phone().address);
}
