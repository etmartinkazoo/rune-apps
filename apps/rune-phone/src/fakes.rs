//! Stand-ins for the phone, for tests: a private D-Bus daemon with PipeWire's telephony service
//! (one phone's audio gateway), obexd's Message and Phonebook Access (a few messages and
//! contacts) and the notification service, each answering as the real ones do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::Connection;
use zbus::Variant;
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant};

pub const PHONE: &str = "AA:BB:CC:DD:EE:FF";
const AG: &str = "/org/pipewire/Telephony/ag0";
const MAP: &str = "/org/bluez/obex/client/session0";
const PBAP: &str = "/org/bluez/obex/client/session1";

/// A dbus-daemon of the tests' own, gone when dropped.
pub struct Bus {
    child: std::process::Child,
    pub address: String,
}

impl Bus {
    pub fn start() -> Bus {
        use std::io::BufRead;
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("dbus-daemon");
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.as_mut().expect("piped")).read_line(&mut line).expect("its address");
        Bus { child, address: line.trim().to_owned() }
    }

    pub async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str()).build().await.expect("connected")
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// What the fakes saw and hold.
#[derive(Default)]
pub struct Seen {
    pub dialed: Vec<String>,
    pub answered: Vec<String>,
    pub hung_up: Vec<String>,
    pub tones: Vec<String>,
    pub pushed: Vec<String>,
    pub notified: Vec<(String, String)>,
    /// The phone's messages: (path, folder, properties as listed, bMessage).
    pub messages: Vec<(String, String, HashMap<String, OwnedVariant>, String)>,
    pub calls: Vec<String>,
    /// Messages marked read (true) or unread on the phone.
    pub read: Vec<(String, bool)>,
}

pub type Shared = Arc<Mutex<Seen>>;

fn props(pairs: &[(&str, Variant)]) -> HashMap<String, OwnedVariant> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), OwnedVariant::try_from(v.try_clone().expect("plain")).expect("owned"))).collect()
}

struct Manager;

#[zbus::interface(name = "org.ofono.Manager")]
impl Manager {
    /// As PipeWire answers: a dictionary, the modem's properties empty (the address is on the
    /// gateway's own interface).
    fn get_modems(&self) -> HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> {
        HashMap::from([(ObjectPath::try_from(AG).expect("path").into(), HashMap::new())])
    }
}

struct Gateway(Shared);

#[zbus::interface(name = "org.ofono.VoiceCallManager")]
impl Gateway {
    fn get_calls(&self) -> HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> {
        HashMap::new()
    }

    async fn dial(&self, number: String, #[zbus(connection)] conn: &Connection) -> zbus::fdo::Result<()> {
        let path = {
            let mut s = self.0.lock().expect("lock");
            s.dialed.push(number.clone());
            format!("{AG}/call{}", s.calls.len())
        };
        add_call(conn, &self.0, &path, &number, "dialing").await;
        Ok(())
    }

    fn send_tones(&self, tones: String) {
        self.0.lock().expect("lock").tones.push(tones);
    }
}

struct Mic {
    level: u8,
}

#[zbus::interface(name = "org.pipewire.Telephony.AudioGateway1")]
impl Mic {
    #[zbus(property)]
    fn address(&self) -> String {
        PHONE.to_owned()
    }

    #[zbus(property)]
    fn microphone_volume(&self) -> u8 {
        self.level
    }

    #[zbus(property)]
    fn set_microphone_volume(&mut self, level: u8) {
        self.level = level;
    }
}

struct CallObject {
    seen: Shared,
    path: String,
}

#[zbus::interface(name = "org.ofono.VoiceCall")]
impl CallObject {
    async fn answer(&self, #[zbus(connection)] conn: &Connection) {
        self.seen.lock().expect("lock").answered.push(self.path.clone());
        let _ = conn.emit_signal(None::<()>, self.path.as_str(), "org.ofono.VoiceCall", "PropertyChanged", &("State", Variant::from("active"))).await;
    }

    async fn hangup(&self, #[zbus(connection)] conn: &Connection) {
        self.seen.lock().expect("lock").hung_up.push(self.path.clone());
        let path = ObjectPath::try_from(self.path.as_str()).expect("path");
        let _ = conn.emit_signal(None::<()>, AG, "org.ofono.VoiceCallManager", "CallRemoved", &(path,)).await;
    }
}

/// A call appears: dialled from here, or ringing on the phone.
pub async fn add_call(conn: &Connection, seen: &Shared, path: &str, number: &str, state: &str) {
    seen.lock().expect("lock").calls.push(path.to_owned());
    conn.object_server().at(path, CallObject { seen: seen.clone(), path: path.to_owned() }).await.expect("served");
    let p = ObjectPath::try_from(path).expect("path");
    let props = props(&[("LineIdentification", Variant::from(number)), ("State", Variant::from(state)), ("Name", Variant::from(""))]);
    conn.emit_signal(None::<()>, AG, "org.ofono.VoiceCallManager", "CallAdded", &(p, props)).await.expect("signal");
}

struct Client;

#[zbus::interface(name = "org.bluez.obex.Client1")]
impl Client {
    fn create_session(&self, destination: String, args: HashMap<String, OwnedVariant>) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(destination, PHONE);
        let target = match args.get("Target").map(|v| &**v) {
            Some(Variant::Str(s)) => s.to_string(),
            _ => String::new(),
        };
        let path = if target == "map" { MAP } else { PBAP };
        Ok(ObjectPath::try_from(path).expect("path").into())
    }

    fn remove_session(&self, _session: OwnedObjectPath) {}
}

struct Messages(Shared);

#[zbus::interface(name = "org.bluez.obex.MessageAccess1")]
impl Messages {
    fn set_folder(&self, name: String) -> zbus::fdo::Result<()> {
        if name.trim_matches('/') == "telecom/msg" { Ok(()) } else { Err(zbus::fdo::Error::Failed(format!("no folder {name}"))) }
    }

    fn list_messages(&self, folder: String, _filter: HashMap<String, OwnedVariant>) -> HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> {
        self.0
            .lock()
            .expect("lock")
            .messages
            .iter()
            .filter(|(_, f, _, _)| *f == folder)
            .map(|(p, _, props, _)| (ObjectPath::try_from(p.as_str()).expect("path").into(), props.iter().map(|(k, v)| (k.clone(), v.try_clone().expect("plain"))).collect()))
            .collect()
    }

    fn push_message(&self, sourcefile: String, folder: String, _args: HashMap<String, OwnedVariant>) -> (OwnedObjectPath, HashMap<String, OwnedVariant>) {
        assert_eq!(folder, "outbox");
        let text = std::fs::read_to_string(&sourcefile).expect("the bMessage file");
        self.0.lock().expect("lock").pushed.push(text);
        // A transfer the fake doesn't keep: gone, as obexd drops a finished one.
        (ObjectPath::try_from("/org/bluez/obex/client/session0/transfer0").expect("path").into(), HashMap::new())
    }
}

struct MessageObject {
    seen: Shared,
    path: String,
}

#[zbus::interface(name = "org.bluez.obex.Message1")]
impl MessageObject {
    fn get(&self, targetfile: String, _attachment: bool) -> (OwnedObjectPath, HashMap<String, OwnedVariant>) {
        let s = self.seen.lock().expect("lock");
        let bmsg = s.messages.iter().find(|(p, ..)| *p == self.path).map(|(.., b)| b.clone()).unwrap_or_default();
        std::fs::write(&targetfile, bmsg).expect("written");
        (ObjectPath::try_from(format!("{}/transfer", self.path)).expect("path").into(), HashMap::new())
    }

    #[zbus(property)]
    fn read(&self) -> bool {
        self.seen.lock().expect("lock").read.iter().rev().find(|(p, _)| *p == self.path).is_some_and(|(_, r)| *r)
    }

    #[zbus(property)]
    fn set_read(&mut self, read: bool) {
        self.seen.lock().expect("lock").read.push((self.path.clone(), read));
    }

    /// Write-only on obexd: setting it deletes the message on the phone.
    #[zbus(property)]
    fn deleted(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn set_deleted(&mut self, deleted: bool) {
        if deleted {
            self.seen.lock().expect("lock").messages.retain(|(p, ..)| *p != self.path);
        }
    }
}

/// A message on the phone. `folder` "inbox" (from `number`) or "sent" (to it).
pub async fn add_message(conn: &Connection, seen: &Shared, id: u32, folder: &str, number: &str, text: &str, at: &str) {
    let path = format!("{MAP}/message{id}");
    let (who, address) = if folder == "inbox" { ("Sender", "SenderAddress") } else { ("Recipient", "RecipientAddress") };
    let listed = props(&[
        (who, Variant::from("")),
        (address, Variant::from(number)),
        ("Subject", Variant::from(text.chars().take(255).collect::<String>())),
        ("Timestamp", Variant::from(at)),
        ("Type", Variant::from("sms-gsm")),
        ("Read", Variant::from(false)),
    ]);
    let bmsg = crate::bmsg::compose_sms(number, text);
    seen.lock().expect("lock").messages.push((path.clone(), folder.to_owned(), listed, bmsg));
    // Served under the ObjectManager at /, so InterfacesAdded goes out as obexd's does.
    conn.object_server().at(path.as_str(), MessageObject { seen: seen.clone(), path: path.clone() }).await.expect("served");
}

struct Phonebook;

#[zbus::interface(name = "org.bluez.obex.PhonebookAccess1")]
impl Phonebook {
    fn select(&self, _location: String, _phonebook: String) {}

    fn pull_all(&self, targetfile: String, _filters: HashMap<String, OwnedVariant>) -> (OwnedObjectPath, HashMap<String, OwnedVariant>) {
        let text = if targetfile.ends_with("pb.vcf") {
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nTEL;TYPE=CELL:+15550100\r\nEND:VCARD\r\n"
        } else {
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nTEL:+15550100\r\nX-IRMC-CALL-DATETIME;MISSED:20261031T091500\r\nEND:VCARD\r\n"
        };
        std::fs::write(&targetfile, text).expect("written");
        (ObjectPath::try_from(format!("{PBAP}/transfer")).expect("path").into(), HashMap::new())
    }
}

struct Notifications(Shared);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app: String,
        _replaces: u32,
        _icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, OwnedVariant>,
        _timeout: i32,
    ) -> u32 {
        let mut s = self.0.lock().expect("lock");
        s.notified.push((summary, body));
        s.notified.len() as u32
    }
}

/// The phone's services on `conn`, under their real names.
pub async fn serve(conn: &Connection) -> Shared {
    let seen = Shared::default();
    let server = conn.object_server();
    server.at("/", zbus::fdo::ObjectManager).await.expect("served");
    server.at("/org/pipewire/Telephony", Manager).await.expect("served");
    server.at(AG, Gateway(seen.clone())).await.expect("served");
    server.at(AG, Mic { level: 15 }).await.expect("served");
    server.at("/org/bluez/obex", Client).await.expect("served");
    server.at(MAP, Messages(seen.clone())).await.expect("served");
    server.at(PBAP, Phonebook).await.expect("served");
    server.at("/org/freedesktop/Notifications", Notifications(seen.clone())).await.expect("served");
    for name in ["org.pipewire.Telephony", "org.bluez.obex", "org.freedesktop.Notifications"] {
        conn.request_name(name).await.expect("a name");
    }
    seen
}
