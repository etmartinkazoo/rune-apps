//! Calls through PipeWire's Bluetooth telephony service (session bus, `org.pipewire.Telephony`):
//! with a phone connected over Hands-Free, PipeWire is its headset. Each phone shows up as an
//! audio gateway that can dial; each call as an object to answer or hang up, whose audio
//! PipeWire routes through this computer. It speaks oFono's interfaces, which this uses.
// On macOS the phone thread is `macworker`; this is for Linux (and its tests).
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::HashMap;

use zbus::Connection;
use zbus::message::Message;
use zbus::Variant;
use zbus::{OwnedObjectPath, OwnedVariant};

pub type Result<T> = std::result::Result<T, String>;

pub const SERVICE: &str = "org.pipewire.Telephony";
pub const MANAGER: &str = "/org/pipewire/Telephony";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Active,
    Held,
    Dialing,
    Alerting,
    Incoming,
    Waiting,
    Disconnected,
}

impl State {
    fn parse(s: &str) -> State {
        match s {
            "active" => State::Active,
            "held" => State::Held,
            "dialing" => State::Dialing,
            "alerting" => State::Alerting,
            "incoming" => State::Incoming,
            "waiting" => State::Waiting,
            _ => State::Disconnected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCall {
    pub path: String,
    /// The other party's number, "withheld" when they hide it.
    pub number: String,
    pub name: String,
    pub state: State,
}

type Props = HashMap<String, OwnedVariant>;

fn text(p: &Props, name: &str) -> String {
    match p.get(name).map(|v| &**v) {
        Some(Variant::Str(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn call_of(path: &str, p: &Props) -> LiveCall {
    LiveCall { path: path.to_owned(), number: text(p, "LineIdentification"), name: text(p, "Name"), state: State::parse(&text(p, "State")) }
}

fn failed(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, msg, _) => match name.as_str() {
            "org.pipewire.Telephony.Error.InvalidState" => "The phone can't do that right now".into(),
            _ => msg.unwrap_or_else(|| name.to_string()),
        },
        e => e.to_string(),
    }
}

/// Objects with their properties, as `GetModems` and `GetCalls` answer: oFono sends a list of
/// pairs (`a(oa{sv})`), PipeWire a dictionary (`a{oa{sv}}`). Either is read.
fn objects(reply: &Message) -> Result<Vec<(OwnedObjectPath, Props)>> {
    let body = reply.body();
    if body.signature().to_string().starts_with("a{") {
        let map: HashMap<OwnedObjectPath, Props> = body.deserialize().map_err(|e| e.to_string())?;
        let mut list: Vec<(OwnedObjectPath, Props)> = map.into_iter().collect();
        list.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        Ok(list)
    } else {
        body.deserialize().map_err(|e| e.to_string())
    }
}

/// The connected phones' audio gateways: (object path, Bluetooth address).
pub async fn gateways(session: &Connection) -> Result<Vec<(String, String)>> {
    let reply = session.call_method(Some(SERVICE), MANAGER, Some("org.ofono.Manager"), "GetModems", &()).await.map_err(failed)?;
    let mut out = Vec::new();
    for (p, props) in objects(&reply)? {
        let mut address = text(&props, "Address");
        // PipeWire leaves the modem's properties empty; its own gateway interface has the address.
        if address.is_empty() {
            address = gateway_address(session, p.as_str()).await.unwrap_or_default();
        }
        out.push((p.to_string(), address));
    }
    Ok(out)
}

async fn gateway_address(session: &Connection, path: &str) -> Option<String> {
    let reply = session
        .call_method(Some(SERVICE), path, Some("org.freedesktop.DBus.Properties"), "Get", &("org.pipewire.Telephony.AudioGateway1", "Address"))
        .await
        .ok()?;
    let v: OwnedVariant = reply.body().deserialize().ok()?;
    match &*v {
        Variant::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

pub async fn calls(session: &Connection, gateway: &str) -> Result<Vec<LiveCall>> {
    let reply = session.call_method(Some(SERVICE), gateway, Some("org.ofono.VoiceCallManager"), "GetCalls", &()).await.map_err(failed)?;
    let calls = objects(&reply)?;
    Ok(calls.iter().map(|(p, props)| call_of(p.as_str(), props)).collect())
}

pub async fn dial(session: &Connection, gateway: &str, number: &str) -> Result<()> {
    let number = crate::number::dialable(number).ok_or("That isn't a number to call")?;
    session.call_method(Some(SERVICE), gateway, Some("org.ofono.VoiceCallManager"), "Dial", &(number.as_str(),)).await.map(drop).map_err(failed)
}

pub async fn answer(session: &Connection, call: &str) -> Result<()> {
    session.call_method(Some(SERVICE), call, Some("org.ofono.VoiceCall"), "Answer", &()).await.map(drop).map_err(failed)
}

pub async fn hang_up(session: &Connection, call: &str) -> Result<()> {
    session.call_method(Some(SERVICE), call, Some("org.ofono.VoiceCall"), "Hangup", &()).await.map(drop).map_err(failed)
}

/// Ends every call on the phone: the way out when ending one call is refused (PipeWire may,
/// while it's still dialing or ringing).
pub async fn hang_up_all(session: &Connection, gateway: &str) -> Result<()> {
    session.call_method(Some(SERVICE), gateway, Some("org.ofono.VoiceCallManager"), "HangupAll", &()).await.map(drop).map_err(failed)
}

/// Keypad tones during a call (menus, extensions): `0`-`9`, `*`, `#`.
pub async fn tones(session: &Connection, gateway: &str, tones: &str) -> Result<()> {
    session.call_method(Some(SERVICE), gateway, Some("org.ofono.VoiceCallManager"), "SendTones", &(tones,)).await.map(drop).map_err(failed)
}

/// Mutes or unmutes this computer's microphone on the call (the gateway's microphone gain).
pub async fn mute(session: &Connection, gateway: &str, muted: bool, unmuted_level: u8) -> Result<()> {
    let level = if muted { 0u8 } else { unmuted_level };
    session
        .call_method(Some(SERVICE), gateway, Some("org.freedesktop.DBus.Properties"), "Set", &("org.pipewire.Telephony.AudioGateway1", "MicrophoneVolume", Variant::from(level)))
        .await
        .map(drop)
        .map_err(failed)
}

/// What a signal from the service says changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Gateway { path: String, address: String },
    GatewayGone(String),
    Call(LiveCall),
    CallGone(String),
    /// A call's state, number or name changed.
    CallProperty { path: String, name: String, value: String },
}

/// The match rule for every signal the service sends.
pub fn rule() -> String {
    format!("type='signal',sender='{SERVICE}'")
}

/// Reads one of the service's signals.
pub fn change(msg: &Message) -> Option<Change> {
    let h = msg.header();
    let member = h.member()?.as_str().to_owned();
    let path = h.path()?.as_str().to_owned();
    let body = msg.body();
    match (h.interface()?.as_str(), member.as_str()) {
        ("org.ofono.Manager", "ModemAdded") => {
            let (p, props): (OwnedObjectPath, Props) = body.deserialize().ok()?;
            Some(Change::Gateway { path: p.to_string(), address: text(&props, "Address") })
        }
        // The same news as the object manager tells it, in case oFono's signal is missed.
        ("org.freedesktop.DBus.ObjectManager", "InterfacesAdded") => {
            let (p, ifaces): (OwnedObjectPath, HashMap<String, Props>) = body.deserialize().ok()?;
            let gateway = ifaces.get("org.pipewire.Telephony.AudioGateway1")?;
            Some(Change::Gateway { path: p.to_string(), address: text(gateway, "Address") })
        }
        ("org.freedesktop.DBus.ObjectManager", "InterfacesRemoved") => {
            let (p, ifaces): (OwnedObjectPath, Vec<String>) = body.deserialize().ok()?;
            ifaces.iter().any(|i| i == "org.pipewire.Telephony.AudioGateway1").then(|| Change::GatewayGone(p.to_string()))
        }
        ("org.ofono.Manager", "ModemRemoved") => Some(Change::GatewayGone(body.deserialize::<OwnedObjectPath>().ok()?.to_string())),
        ("org.ofono.VoiceCallManager", "CallAdded") => {
            let (p, props): (OwnedObjectPath, Props) = body.deserialize().ok()?;
            Some(Change::Call(call_of(p.as_str(), &props)))
        }
        ("org.ofono.VoiceCallManager", "CallRemoved") => Some(Change::CallGone(body.deserialize::<OwnedObjectPath>().ok()?.to_string())),
        ("org.ofono.VoiceCall", "PropertyChanged") => {
            let (name, value): (String, OwnedVariant) = body.deserialize().ok()?;
            let value = match &*value {
                Variant::Str(s) => s.to_string(),
                other => format!("{other}"),
            };
            Some(Change::CallProperty { path, name, value })
        }
        _ => None,
    }
}

/// Applies a property change to the calls known.
pub fn apply(calls: &mut [LiveCall], path: &str, name: &str, value: &str) {
    if let Some(c) = calls.iter_mut().find(|c| c.path == path) {
        match name {
            "State" => c.state = State::parse(value),
            "LineIdentification" => c.number = value.to_owned(),
            "Name" => c.name = value.to_owned(),
            _ => {}
        }
    }
}
