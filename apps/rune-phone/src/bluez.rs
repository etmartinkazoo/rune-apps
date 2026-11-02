//! BlueZ on the system bus: the phones this computer knows, connecting one, and pairing a new
//! one, with an agent that shows the six-digit code both screens must agree on.
// On macOS the phone thread is `macworker`; this is for Linux (and its tests).
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use zbus::Connection;
use zbus::Variant;
use zbus::{OwnedObjectPath, OwnedVariant};

pub type Result<T> = std::result::Result<T, String>;

/// Services a phone offers that this app uses, by their UUIDs' distinctive part.
const HANDSFREE_GATEWAY: &str = "0000111f";
const MESSAGE_SERVER: &str = "00001132";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub path: String,
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    /// It looks like a phone: its icon says so, or it offers calls or messages.
    pub phone: bool,
}

type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>>;

fn prop<'a>(props: &'a HashMap<String, OwnedVariant>, name: &str) -> Option<&'a Variant<'static>> {
    props.get(name).map(|v| &**v)
}

fn text(props: &HashMap<String, OwnedVariant>, name: &str) -> String {
    match prop(props, name) {
        Some(Variant::Str(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn flag(props: &HashMap<String, OwnedVariant>, name: &str) -> bool {
    matches!(prop(props, name), Some(Variant::Bool(true)))
}

pub fn device_of(path: &str, props: &HashMap<String, OwnedVariant>) -> Device {
    let uuids: Vec<String> = match prop(props, "UUIDs") {
        Some(Variant::Array(a)) => a.iter().filter_map(|v| if let Variant::Str(s) = v { Some(s.to_lowercase()) } else { None }).collect(),
        _ => Vec::new(),
    };
    let offers = |id: &str| uuids.iter().any(|u| u.starts_with(id));
    let name = Some(text(props, "Alias")).filter(|s| !s.is_empty()).unwrap_or_else(|| text(props, "Name"));
    Device {
        path: path.to_owned(),
        address: text(props, "Address"),
        name,
        paired: flag(props, "Paired"),
        connected: flag(props, "Connected"),
        phone: text(props, "Icon") == "phone" || offers(HANDSFREE_GATEWAY) || offers(MESSAGE_SERVER),
    }
}

/// Every device BlueZ knows (paired, or seen while discovering).
pub async fn devices(system: &Connection) -> Result<Vec<Device>> {
    let reply = system
        .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .await
        .map_err(|e| format!("Bluetooth isn't available: {e}"))?;
    let objects: Managed = reply.body().deserialize().map_err(|e| e.to_string())?;
    let mut out: Vec<Device> = objects
        .iter()
        .filter_map(|(path, ifaces)| ifaces.get("org.bluez.Device1").map(|p| device_of(path.as_str(), p)))
        .collect();
    out.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.name.cmp(&b.name)));
    Ok(out)
}

/// The first adapter's path.
async fn adapter(system: &Connection) -> Result<String> {
    let reply = system
        .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .await
        .map_err(|e| e.to_string())?;
    let objects: Managed = reply.body().deserialize().map_err(|e| e.to_string())?;
    let mut paths: Vec<String> = objects.iter().filter(|(_, i)| i.contains_key("org.bluez.Adapter1")).map(|(p, _)| p.to_string()).collect();
    paths.sort();
    paths.into_iter().next().ok_or_else(|| "This computer has no Bluetooth adapter".into())
}

/// Looks for devices nearby (`on`), or stops.
pub async fn discover(system: &Connection, on: bool) -> Result<()> {
    let a = adapter(system).await?;
    let method = if on { "StartDiscovery" } else { "StopDiscovery" };
    if on {
        let mut filter: HashMap<&str, Variant> = HashMap::new();
        filter.insert("Transport", Variant::from("bredr"));
        let _ = system.call_method(Some("org.bluez"), a.as_str(), Some("org.bluez.Adapter1"), "SetDiscoveryFilter", &(filter,)).await;
        let _ = system
            .call_method(Some("org.bluez"), a.as_str(), Some("org.freedesktop.DBus.Properties"), "Set", &("org.bluez.Adapter1", "Powered", Variant::from(true)))
            .await;
    }
    system.call_method(Some("org.bluez"), a.as_str(), Some("org.bluez.Adapter1"), method, &()).await.map(drop).map_err(|e| e.to_string())
}

async fn device_call(system: &Connection, path: &str, method: &str) -> Result<()> {
    system.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), method, &()).await.map(drop).map_err(|e| match e {
        zbus::Error::MethodError(name, msg, _) => msg.unwrap_or_else(|| name.to_string()),
        e => e.to_string(),
    })
}

/// Pairs with the device at `path` (the agent asks the UI to confirm the code), trusts it so it
/// reconnects on its own, and connects it.
pub async fn pair(system: &Connection, path: &str) -> Result<()> {
    device_call(system, path, "Pair").await?;
    let _ = system
        .call_method(Some("org.bluez"), path, Some("org.freedesktop.DBus.Properties"), "Set", &("org.bluez.Device1", "Trusted", Variant::from(true)))
        .await;
    connect(system, path).await
}

/// Forgets the device at `path`: BlueZ disconnects it and drops the pairing, so using it again
/// means pairing again.
pub async fn remove(system: &Connection, path: &str) -> Result<()> {
    let adapter = path.rsplit_once('/').map(|(a, _)| a).filter(|a| !a.is_empty()).ok_or("Not a Bluetooth device")?;
    let device = zbus::ObjectPath::try_from(path).map_err(|e| e.to_string())?;
    system.call_method(Some("org.bluez"), adapter, Some("org.bluez.Adapter1"), "RemoveDevice", &(device,)).await.map(drop).map_err(|e| match e {
        zbus::Error::MethodError(name, msg, _) => msg.unwrap_or_else(|| name.to_string()),
        e => e.to_string(),
    })
}

/// Connects the phone's profiles (calls through PipeWire; messages are an OBEX session apart).
pub async fn connect(system: &Connection, path: &str) -> Result<()> {
    match device_call(system, path, "Connect").await {
        Err(e) if e.contains("Already Connected") || e.contains("AlreadyConnected") => Ok(()),
        r => r,
    }
}

/// A pairing code waiting for the person to confirm on both screens.
pub struct Confirmation {
    pub device: String,
    pub passkey: u32,
    pub answer: oneshot::Sender<bool>,
}

/// The pairing agent: hands each code to the app and waits for its answer.
pub struct Agent {
    pub requests: tokio::sync::mpsc::UnboundedSender<Confirmation>,
    /// The device being paired from here; anything else asking is turned away.
    pub expecting: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum AgentError {
    Rejected(String),
    Canceled(String),
}

impl Agent {
    fn expected(&self, device: &str) -> bool {
        self.expecting.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(device)
    }

    async fn ask(&self, device: &str, passkey: u32) -> std::result::Result<(), AgentError> {
        if !self.expected(device) {
            return Err(AgentError::Rejected("Not pairing with this device".into()));
        }
        let (answer, wait) = oneshot::channel();
        self.requests
            .send(Confirmation { device: device.to_owned(), passkey, answer })
            .map_err(|_| AgentError::Canceled("The app closed".into()))?;
        match tokio::time::timeout(std::time::Duration::from_secs(60), wait).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(AgentError::Rejected("Codes didn't match".into())),
            _ => Err(AgentError::Canceled("No answer".into())),
        }
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    async fn request_confirmation(&self, device: OwnedObjectPath, passkey: u32) -> std::result::Result<(), AgentError> {
        self.ask(device.as_str(), passkey).await
    }

    /// Phones pair with numeric comparison; a just-works request from the phone being paired is
    /// accepted.
    async fn request_authorization(&self, device: OwnedObjectPath) -> std::result::Result<(), AgentError> {
        if self.expected(device.as_str()) { Ok(()) } else { Err(AgentError::Rejected("Not pairing".into())) }
    }

    async fn authorize_service(&self, device: OwnedObjectPath, _uuid: String) -> std::result::Result<(), AgentError> {
        self.request_authorization(device).await
    }

    async fn request_pin_code(&self, _device: OwnedObjectPath) -> std::result::Result<String, AgentError> {
        Err(AgentError::Rejected("PIN codes aren't used for phones".into()))
    }

    async fn request_passkey(&self, _device: OwnedObjectPath) -> std::result::Result<u32, AgentError> {
        Err(AgentError::Rejected("Passkey entry isn't used for phones".into()))
    }

    async fn display_passkey(&self, _device: OwnedObjectPath, _passkey: u32, _entered: u16) {}

    async fn display_pin_code(&self, _device: OwnedObjectPath, _pincode: String) {}

    async fn cancel(&self) {}

    async fn release(&self) {}
}

pub const AGENT_PATH: &str = "/org/runephone/agent";

/// Serves the agent and registers it with BlueZ, on a connection of its own.
pub async fn serve_agent(agent: Agent) -> Result<Connection> {
    let conn = zbus::connection::Builder::system()
        .serve_at(AGENT_PATH, agent)
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let path = zbus::ObjectPath::try_from(AGENT_PATH).expect("a valid path");
    conn.call_method(Some("org.bluez"), "/org/bluez", Some("org.bluez.AgentManager1"), "RegisterAgent", &(&path, "DisplayYesNo"))
        .await
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, Variant<'static>)]) -> HashMap<String, OwnedVariant> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), OwnedVariant::try_from(v.try_clone().unwrap()).unwrap())).collect()
    }

    #[test]
    fn tells_a_phone_from_a_mouse() {
        let pixel = device_of(
            "/org/bluez/hci0/dev_AA",
            &props(&[
                ("Address", Variant::from("AA:BB:CC:DD:EE:FF")),
                ("Name", Variant::from("Pixel 9")),
                ("Paired", Variant::from(true)),
                ("UUIDs", Variant::from(vec!["0000111f-0000-1000-8000-00805f9b34fb"])),
            ]),
        );
        assert!(pixel.phone && pixel.paired && !pixel.connected);
        assert_eq!(pixel.name, "Pixel 9");
        let mouse = device_of("/x", &props(&[("Name", Variant::from("LIFT")), ("Icon", Variant::from("input-mouse"))]));
        assert!(!mouse.phone);
    }
}
