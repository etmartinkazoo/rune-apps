//! Messages and contacts through obexd (session bus, `org.bluez.obex`): a Message Access
//! session lists, reads and sends the phone's SMS and MMS (Android asks once whether to allow
//! it), and a Phonebook Access session pulls its contacts and call history. Files move as
//! transfers: obexd writes what it reads to a file, and sends what's in one.
// On macOS the phone thread is `macworker`; this is for Linux (and its tests).
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use zbus::Connection;
use zbus::Variant;
use zbus::{OwnedObjectPath, OwnedVariant};

pub type Result<T> = std::result::Result<T, String>;

pub const SERVICE: &str = "org.bluez.obex";
const CLIENT: &str = "/org/bluez/obex";

type Props = HashMap<String, OwnedVariant>;

fn failed(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, msg, _) => {
            let msg = msg.unwrap_or_else(|| name.to_string());
            if msg.contains("Forbidden") || msg.contains("Unauthorized") || name.as_str().ends_with("NotAuthorized") {
                "The phone didn't allow it: on the phone, allow access in this computer's Bluetooth settings".into()
            } else {
                msg
            }
        }
        e => e.to_string(),
    }
}

fn text(p: &Props, name: &str) -> String {
    match p.get(name).map(|v| &**v) {
        Some(Variant::Str(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn flag(p: &Props, name: &str) -> bool {
    matches!(p.get(name).map(|v| &**v), Some(Variant::Bool(true)))
}

/// Opens a session of `target` ("map" or "pbap") with the phone at `address`.
pub async fn session(session: &Connection, address: &str, target: &str) -> Result<String> {
    let mut args: HashMap<&str, Variant> = HashMap::new();
    args.insert("Target", Variant::from(target));
    let reply = session.call_method(Some(SERVICE), CLIENT, Some("org.bluez.obex.Client1"), "CreateSession", &(address, args)).await.map_err(failed)?;
    Ok(reply.body().deserialize::<OwnedObjectPath>().map_err(|e| e.to_string())?.to_string())
}

pub async fn close(session: &Connection, path: &str) {
    if let Ok(p) = zbus::ObjectPath::try_from(path) {
        let _ = session.call_method(Some(SERVICE), CLIENT, Some("org.bluez.obex.Client1"), "RemoveSession", &(p,)).await;
    }
}

/// Waits for a transfer to finish; obexd drops its object once done, which counts as done.
async fn finish(session: &Connection, transfer: &str) -> Result<()> {
    for _ in 0..600 {
        let reply = session
            .call_method(Some(SERVICE), transfer, Some("org.freedesktop.DBus.Properties"), "Get", &("org.bluez.obex.Transfer1", "Status"))
            .await;
        match reply {
            Ok(r) => {
                let v: OwnedVariant = r.body().deserialize().map_err(|e| e.to_string())?;
                match &*v {
                    Variant::Str(s) if s.as_str() == "complete" => return Ok(()),
                    Variant::Str(s) if s.as_str() == "error" => return Err("The transfer failed".into()),
                    _ => {}
                }
            }
            // Gone: complete (obexd removes a transfer once it has ended).
            Err(_) => return Ok(()),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("The phone took too long".into())
}

/// Where transferred files go: this user's runtime directory, readable by obexd (which runs
/// as this user) and by nobody else.
pub fn scratch() -> PathBuf {
    let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir).join("rune-phone");
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

/// Reads a finished transfer's file and removes it.
fn take(path: &Path) -> Result<String> {
    let text = std::fs::read(path).map_err(|e| format!("Couldn't read what the phone sent: {e}"))?;
    let _ = std::fs::remove_file(path);
    Ok(String::from_utf8_lossy(&text).into_owned())
}

// Messages

/// A message as the phone lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub path: String,
    /// "inbox", "sent", "outbox"…
    pub folder: String,
    /// The other party: who sent it, or who it went to.
    pub name: String,
    pub number: String,
    /// The text as listed (the phone fills this with the message, up to 256 characters).
    pub preview: String,
    /// `YYYYMMDDTHHMMSS`, local time.
    pub at: String,
    pub read: bool,
    pub incoming: bool,
    pub mms: bool,
}

/// Moves a MAP session into the message folders.
pub async fn open_messages(session: &Connection, map: &str) -> Result<()> {
    session.call_method(Some(SERVICE), map, Some("org.bluez.obex.MessageAccess1"), "SetFolder", &("/telecom/msg",)).await.map(drop).map_err(failed)
}

/// The newest `count` messages in `folder`.
pub async fn list(session: &Connection, map: &str, folder: &str, count: u16) -> Result<Vec<Listed>> {
    let mut filter: HashMap<&str, Variant> = HashMap::new();
    filter.insert("MaxCount", Variant::from(count));
    filter.insert("SubjectLength", Variant::from(255u8));
    let reply = session
        .call_method(Some(SERVICE), map, Some("org.bluez.obex.MessageAccess1"), "ListMessages", &(folder, filter))
        .await
        .map_err(failed)?;
    let items: HashMap<OwnedObjectPath, Props> = reply.body().deserialize().map_err(|e| e.to_string())?;
    let incoming = folder == "inbox";
    let mut out: Vec<Listed> = items
        .iter()
        .map(|(path, p)| {
            let (name, number) = if incoming { (text(p, "Sender"), text(p, "SenderAddress")) } else { (text(p, "Recipient"), text(p, "RecipientAddress")) };
            let number = crate::number::address(&number);
            Listed {
                path: path.to_string(),
                folder: folder.to_owned(),
                name,
                number,
                preview: text(p, "Subject"),
                at: text(p, "Timestamp"),
                read: flag(p, "Read") || !incoming,
                incoming,
                // MAP 1.3's instant messages ("im") come as MIME, like an MMS.
                mms: matches!(text(p, "Type").as_str(), "mms" | "im"),
            }
        })
        .collect();
    out.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(out)
}

/// A message whole, as a bMessage.
pub async fn get(session: &Connection, message: &str) -> Result<crate::bmsg::Parsed> {
    let file = scratch().join(format!("msg-{}", message.rsplit('/').next().unwrap_or("m")));
    let target = file.to_string_lossy().into_owned();
    let reply = session.call_method(Some(SERVICE), message, Some("org.bluez.obex.Message1"), "Get", &(target.as_str(), false)).await.map_err(failed)?;
    let (transfer, _): (OwnedObjectPath, Props) = reply.body().deserialize().map_err(|e| e.to_string())?;
    finish(session, transfer.as_str()).await?;
    Ok(crate::bmsg::parse(&take(&file)?))
}

/// Marks a message read (or unread) on the phone.
pub async fn set_read(session: &Connection, message: &str, read: bool) -> Result<()> {
    session
        .call_method(Some(SERVICE), message, Some("org.freedesktop.DBus.Properties"), "Set", &("org.bluez.obex.Message1", "Read", Variant::from(read)))
        .await
        .map(drop)
        .map_err(failed)
}

/// Deletes a message on the phone.
pub async fn delete(session: &Connection, message: &str) -> Result<()> {
    session
        .call_method(Some(SERVICE), message, Some("org.freedesktop.DBus.Properties"), "Set", &("org.bluez.obex.Message1", "Deleted", Variant::from(true)))
        .await
        .map(drop)
        .map_err(failed)
}

/// Sends `text` to `number` as an SMS.
pub async fn send(session: &Connection, map: &str, number: &str, text: &str) -> Result<()> {
    let file = scratch().join(format!("send-{}.bmsg", std::process::id()));
    std::fs::write(&file, crate::bmsg::compose_sms(number, text)).map_err(|e| e.to_string())?;
    let target = file.to_string_lossy().into_owned();
    let mut args: HashMap<&str, Variant> = HashMap::new();
    args.insert("Charset", Variant::from("utf8"));
    let result = async {
        let reply = session
            .call_method(Some(SERVICE), map, Some("org.bluez.obex.MessageAccess1"), "PushMessage", &(target.as_str(), "outbox", args))
            .await
            .map_err(failed)?;
        let (transfer, _): (OwnedObjectPath, Props) = reply.body().deserialize().map_err(|e| e.to_string())?;
        finish(session, transfer.as_str()).await
    }
    .await;
    let _ = std::fs::remove_file(&file);
    result
}

/// The match rule for new messages: obexd adds an object for each one the phone reports.
pub fn new_message_rule() -> String {
    format!("type='signal',sender='{SERVICE}',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesAdded'")
}

/// Whether a signal announces a message (in session `map`).
pub fn is_new_message(msg: &zbus::message::Message, map: &str) -> bool {
    let Ok((path, ifaces)) = msg.body().deserialize::<(OwnedObjectPath, HashMap<String, Props>)>() else { return false };
    path.as_str().starts_with(map) && ifaces.contains_key("org.bluez.obex.Message1")
}

// Contacts and calls

/// One of the phone's phonebooks as vCard text: "pb" (contacts) or "cch" (call history).
pub async fn phonebook(session: &Connection, pbap: &str, book: &str) -> Result<String> {
    session.call_method(Some(SERVICE), pbap, Some("org.bluez.obex.PhonebookAccess1"), "Select", &("int", book)).await.map_err(failed)?;
    let file = scratch().join(format!("{book}.vcf"));
    let target = file.to_string_lossy().into_owned();
    let mut filters: HashMap<&str, Variant> = HashMap::new();
    filters.insert("Format", Variant::from("vcard30"));
    let reply = session
        .call_method(Some(SERVICE), pbap, Some("org.bluez.obex.PhonebookAccess1"), "PullAll", &(target.as_str(), filters))
        .await
        .map_err(failed)?;
    let (transfer, _): (OwnedObjectPath, Props) = reply.body().deserialize().map_err(|e| e.to_string())?;
    finish(session, transfer.as_str()).await?;
    take(&file)
}
