//! The phone thread on macOS: the same commands and events as on Linux (`worker`), with
//! IOBluetooth (`macbt`) for the link and the app's own OBEX (`goep`, `profiles`) for messages
//! and contacts. macOS gives apps no hands-free audio, so calls stay on the phone. And it has no
//! way for the phone to announce a new text, so the inbox is listed again every little while.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use rune::Waker;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::bluez::Device;
use crate::goep::Client;
use crate::macbt::{self, Inquiry, Pairing, Rfcomm};
use crate::profiles;
use crate::vcard;
use crate::worker::{Cmd, Event, Link, Message};

/// How many of the newest messages each folder lists.
const MESSAGES: u16 = 400;
/// New texts are looked for this often.
const RELIST: Duration = Duration::from_secs(30);
/// With the phone out of reach, it's tried again this often.
const RETRY: Duration = Duration::from_secs(60);
/// How long a search for phones goes on.
const SEARCH: Duration = Duration::from_secs(12);

const NO_CALLS: &str = "Calls stay on the phone on a Mac: macOS doesn't let apps carry call audio";

pub fn spawn(wake: Waker, phone: Option<String>) -> (UnboundedSender<Cmd>, std::sync::mpsc::Receiver<Event>) {
    let (cmds, rx) = unbounded_channel();
    let (tx, events) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("phone".into())
        .spawn(move || Worker::new(tx, wake).run(rx, phone))
        .expect("spawn the phone thread");
    (cmds, events)
}

/// `rune-phone --probe [ADDRESS]`: the phone thread without the window, printing what it hands
/// over, for seeing what a phone allows. With no address, it lists the paired devices.
pub fn probe(address: Option<String>) {
    use objc2_foundation::{NSDate, NSRunLoop};
    let (cmds, events) = spawn(std::sync::Arc::new(|| {}), address.clone());
    let started = Instant::now();
    let mut seen = (false, false);
    let mut newest: Vec<String> = Vec::new();
    let mut fetched = false;
    // The main thread serves the run loop, as the window's would: IOBluetooth lives on it.
    while started.elapsed() < Duration::from_secs(300) {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.1));
        for e in events.try_iter() {
            match e {
                Event::Devices(d) => {
                    for d in &d {
                        println!("device {} {} paired={} phone={}", d.address, d.name, d.paired, d.phone);
                    }
                    if address.is_none() {
                        return;
                    }
                }
                Event::Link(l) => println!("link messages={} contacts={} calls={}", l.messages, l.contacts, l.calls),
                Event::Messages(m) => {
                    println!("messages {}", m.len());
                    newest = m.iter().rev().take(3).map(|m| m.id.clone()).collect();
                    seen.0 = true;
                }
                Event::Contacts(c) => {
                    println!("contacts {}", c.len());
                    seen.1 = true;
                }
                Event::Recents(r) => println!("recent calls {}", r.len()),
                Event::Fetched(f) => {
                    for (id, text, attachment) in &f {
                        println!("fetched {id}: {} characters{}", text.chars().count(), if *attachment { ", with an attachment" } else { "" });
                    }
                    if f.is_empty() {
                        println!("fetched nothing");
                    }
                    if fetched {
                        return;
                    }
                    fetched = true;
                    // Whether the session survived: the same again (it only reads).
                    let _ = cmds.send(Cmd::Fetch(f.iter().map(|(id, ..)| id.clone()).collect()));
                }
                Event::Error(e) => {
                    println!("error: {e}");
                    if fetched {
                        return;
                    }
                }
                _ => {}
            }
        }
        if seen.0 && seen.1 && !newest.is_empty() {
            let _ = cmds.send(Cmd::Fetch(std::mem::take(&mut newest)));
        }
    }
    drop(cmds);
}

struct Worker {
    events: std::sync::mpsc::Sender<Event>,
    wake: Waker,
    link: Link,
    map: Option<Client<Rfcomm>>,
    /// The phonebook session, kept open once made: closing a channel to the phone can make
    /// macOS close the message session's channel with it.
    pbap: Option<Client<Rfcomm>>,
    messages: Vec<Message>,
    listed: Instant,
    /// When reaching the phone last failed.
    failed: Option<Instant>,
    search: Option<(Inquiry, Instant)>,
    pairing: Option<(String, Pairing)>,
}

impl Worker {
    fn new(events: std::sync::mpsc::Sender<Event>, wake: Waker) -> Self {
        Worker {
            events,
            wake,
            link: Link::default(),
            map: None,
            pbap: None,
            messages: Vec::new(),
            listed: Instant::now(),
            failed: None,
            search: None,
            pairing: None,
        }
    }

    fn run(mut self, mut cmds: UnboundedReceiver<Cmd>, phone: Option<String>) {
        // Only for waiting on the next command with a timeout.
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("a tokio runtime");
        self.devices();
        if let Some(address) = phone {
            self.use_phone(&address);
        }
        loop {
            let wait = if self.search.is_some() || self.pairing.is_some() { Duration::from_millis(250) } else { Duration::from_secs(1) };
            match rt.block_on(async { tokio::time::timeout(wait, cmds.recv()).await }) {
                Ok(Some(cmd)) => self.handle(cmd),
                Ok(None) => return,
                Err(_) => {}
            }
            self.tick();
        }
    }

    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn error(&self, e: impl Into<String>) {
        self.send(Event::Error(e.into()));
    }

    /// What happens between commands: a search's finds, a pairing's progress, new texts.
    fn tick(&mut self) {
        if let Some((inquiry, started)) = &self.search {
            let found = inquiry.found();
            let done = started.elapsed() > SEARCH;
            self.devices_with(found);
            if done {
                self.search = None;
            }
        }
        if let Some((address, pairing)) = &self.pairing {
            if let Some(passkey) = pairing.code() {
                let name = self.name_of(address);
                self.send(Event::PairCode { name, passkey });
            }
            if let Some(result) = pairing.finished() {
                let address = address.clone();
                self.pairing = None;
                self.pair_done(&address, result);
            }
        }
        let Some(address) = self.link.phone.as_ref().map(|p| p.address.clone()) else { return };
        if self.map.is_some() {
            if self.listed.elapsed() > RELIST {
                self.relist(20);
            }
        } else if self.failed.is_some_and(|t| t.elapsed() > RETRY) {
            // Back in reach, perhaps.
            self.open_messages(&address);
            if self.map.is_some() {
                self.load_contacts(&address);
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        let r = match cmd {
            Cmd::Discover(on) => {
                self.search = None;
                if on {
                    match Inquiry::start() {
                        Ok(i) => {
                            self.search = Some((i, Instant::now()));
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    Ok(())
                }
            }
            Cmd::Pair(address) => {
                self.search = None;
                match Pairing::start(&address) {
                    Ok(p) => {
                        self.pairing = Some((address, p));
                        Ok(())
                    }
                    Err(e) => {
                        self.send(Event::PairFailed);
                        Err(e)
                    }
                }
            }
            // On Linux a pairing reports back through this; here `tick` watches it instead.
            Cmd::PairDone { .. } => Ok(()),
            Cmd::Confirm(yes) => {
                if let Some((_, p)) = &self.pairing {
                    p.answer(yes);
                }
                Ok(())
            }
            Cmd::Use(address) => {
                self.use_phone(&address);
                Ok(())
            }
            Cmd::Remove(address) => self.remove(&address),
            Cmd::Dial(_) | Cmd::Answer => Err(NO_CALLS.into()),
            Cmd::HangUp | Cmd::Tones(_) | Cmd::Mute(_) => Ok(()),
            Cmd::Send { number, text } => self.send_sms(&number, &text),
            Cmd::Fetch(ids) => {
                let mut got = Vec::new();
                for id in ids {
                    match self.map.as_mut().map(|map| profiles::get(map, &id)) {
                        Some(Ok(p)) => got.push((id, p.text, p.attachment)),
                        // The phone dropped the session over it: open another for what comes
                        // next (a reply, most likely), and leave the rest as listed.
                        Some(Err(e)) if gone(&e) => {
                            self.map = self.link.phone.as_ref().and_then(|p| connect_messages(&p.address).ok());
                            break;
                        }
                        _ => {}
                    }
                }
                self.send(Event::Fetched(got));
                Ok(())
            }
            Cmd::MarkRead(ids) => self.set_read(&ids, true),
            Cmd::MarkUnread(ids) => self.set_read(&ids, false),
            Cmd::Delete(ids) => self.delete(&ids),
        };
        if let Err(e) = r {
            self.error(e);
        }
    }

    fn devices(&self) {
        self.devices_with(Vec::new());
    }

    /// The paired devices, and any others a search has found.
    fn devices_with(&self, found: Vec<Device>) {
        let mut list = macbt::paired();
        for d in found {
            if !list.iter().any(|k| k.address == d.address) {
                list.push(d);
            }
        }
        list.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.name.cmp(&b.name)));
        self.send(Event::Devices(list));
    }

    fn name_of(&self, address: &str) -> String {
        let known = macbt::paired().into_iter().chain(self.search.iter().flat_map(|(i, _)| i.found()));
        known.into_iter().find(|d| d.address == address).map(|d| d.name).unwrap_or_default()
    }

    fn pair_done(&mut self, address: &str, result: i32) {
        if result != 0 {
            self.send(Event::PairFailed);
            return self.error(format!("Pairing didn't work (IOBluetooth 0x{result:08X})"));
        }
        let device = macbt::paired().into_iter().find(|d| d.address == address).unwrap_or(Device {
            path: address.to_owned(),
            address: address.to_owned(),
            name: "Phone".into(),
            paired: true,
            connected: true,
            phone: true,
        });
        self.devices();
        self.send(Event::Paired(device));
        self.use_phone(address);
    }

    fn remove(&mut self, address: &str) -> Result<(), String> {
        let device = macbt::paired().into_iter().find(|d| d.address == address).ok_or("That phone is already gone")?;
        if self.link.phone.as_ref().is_some_and(|p| p.address == address) {
            self.drop_phone();
        }
        let r = macbt::remove(address);
        self.devices();
        r?;
        self.send(Event::Removed(device));
        Ok(())
    }

    fn drop_phone(&mut self) {
        if let Some(mut map) = self.map.take() {
            map.disconnect();
        }
        if let Some(mut pbap) = self.pbap.take() {
            pbap.disconnect();
        }
        self.messages.clear();
        self.link = Link::default();
        self.failed = None;
        self.send(Event::Calls(Vec::new()));
        self.send(Event::Link(self.link.clone()));
    }

    /// Uses the phone at `address`: its messages, then its contacts and recent calls.
    fn use_phone(&mut self, address: &str) {
        if let Some(mut old) = self.map.take() {
            old.disconnect();
        }
        if let Some(mut old) = self.pbap.take() {
            old.disconnect();
        }
        let phone = macbt::paired().into_iter().find(|d| d.address.eq_ignore_ascii_case(address)).unwrap_or(Device {
            path: address.to_owned(),
            address: address.to_owned(),
            name: "Phone".into(),
            paired: true,
            connected: false,
            phone: true,
        });
        self.messages.clear();
        self.link = Link { phone: Some(phone), ..Link::default() };
        self.send(Event::Calls(Vec::new()));
        self.send(Event::Link(self.link.clone()));
        self.open_messages(address);
        self.load_contacts(address);
    }

    fn open_messages(&mut self, address: &str) {
        match connect_messages(address) {
            Ok(map) => {
                self.map = Some(map);
                self.failed = None;
                self.link.messages = true;
                if let Some(p) = self.link.phone.as_mut() {
                    p.connected = true;
                }
                self.send(Event::Link(self.link.clone()));
                self.relist(MESSAGES);
            }
            Err(e) => {
                self.failed = Some(Instant::now());
                self.error(format!("Messages: {e}"));
            }
        }
    }

    /// Lists the newest `count` of the inbox and sent folders, merged into what's known.
    fn relist(&mut self, count: u16) {
        self.listed = Instant::now();
        let Some(map) = self.map.as_mut() else { return };
        let mut fresh: Vec<Message> = Vec::new();
        for folder in ["inbox", "sent"] {
            match profiles::list(map, folder, count) {
                Ok(l) => fresh.extend(l.iter().map(Message::from)),
                Err(e) => {
                    // Out of reach: open another session once it's back.
                    self.map = None;
                    self.failed = Some(Instant::now());
                    self.link.messages = false;
                    if let Some(p) = self.link.phone.as_mut() {
                        p.connected = false;
                    }
                    self.send(Event::Link(self.link.clone()));
                    self.error(format!("Messages: {e}"));
                    return;
                }
            }
        }
        let known: HashSet<String> = self.messages.iter().map(|m| m.id.clone()).collect();
        let first = self.messages.is_empty();
        if !first {
            for m in fresh.iter().filter(|m| m.incoming && !m.read && !known.contains(&m.id)) {
                let who = if m.name.is_empty() { crate::number::pretty(&m.number) } else { m.name.clone() };
                notify(&who, &m.text);
            }
        }
        for m in fresh {
            match self.messages.iter_mut().find(|k| k.id == m.id) {
                Some(k) => {
                    // A fetched text stays, unless the listing has more.
                    let text = if k.text.len() > m.text.len() { k.text.clone() } else { m.text.clone() };
                    *k = Message { text, partial: k.partial && m.partial, ..m };
                }
                None => self.messages.push(m),
            }
        }
        self.messages.sort_by(|a, b| a.at.cmp(&b.at));
        self.send(Event::Messages(self.messages.clone()));
    }

    /// Runs `op` on the message session. The phone may have closed it (it drops the session
    /// over a request it can't handle, and when out of reach): then it's opened again and `op`
    /// tried once more.
    fn on_messages<T>(&mut self, mut op: impl FnMut(&mut Client<Rfcomm>) -> Result<T, String>) -> Result<T, String> {
        let address = self.link.phone.as_ref().map(|p| p.address.clone()).ok_or("Messages need the phone connected over Bluetooth")?;
        if let Some(map) = self.map.as_mut() {
            match op(map) {
                Err(e) if gone(&e) => {}
                r => return r,
            }
        }
        self.map = None;
        let mut map = connect_messages(&address).map_err(|e| {
            self.failed = Some(Instant::now());
            e
        })?;
        let r = op(&mut map);
        self.map = Some(map);
        self.failed = None;
        r
    }

    fn set_read(&mut self, ids: &[String], read: bool) -> Result<(), String> {
        let mut failed = 0;
        for id in ids {
            match self.on_messages(|map| profiles::set_read(map, id, read)) {
                Ok(()) => self.messages.iter_mut().filter(|m| &m.id == id).for_each(|m| m.read = read),
                Err(_) => failed += 1,
            }
        }
        if failed > 0 {
            self.send(Event::Messages(self.messages.clone()));
            return Err(format!("The phone didn't mark {failed} of them {}", if read { "read" } else { "unread" }));
        }
        Ok(())
    }

    fn delete(&mut self, ids: &[String]) -> Result<(), String> {
        let mut failed = 0;
        for id in ids {
            match self.on_messages(|map| profiles::delete(map, id)) {
                Ok(()) => self.messages.retain(|m| &m.id != id),
                Err(_) => failed += 1,
            }
        }
        self.send(Event::Messages(self.messages.clone()));
        if failed > 0 {
            return Err(format!("The phone didn't delete {failed} of {}", ids.len()));
        }
        Ok(())
    }

    fn send_sms(&mut self, number: &str, text: &str) -> Result<(), String> {
        self.on_messages(|map| profiles::send(map, number, text))?;
        self.send(Event::Sent);
        self.relist(20);
        Ok(())
    }

    fn load_contacts(&mut self, address: &str) {
        let pull = |c: &mut Client<Rfcomm>| -> Result<(String, Option<String>), String> {
            let pb = profiles::phonebook(c, "pb")?;
            Ok((pb, profiles::phonebook(c, "cch").ok()))
        };
        // The open session if it still works, else a new one.
        let mut r = match self.pbap.as_mut() {
            Some(c) => pull(c),
            None => Err(crate::goep::GONE.into()),
        };
        if r.as_ref().is_err_and(|e| gone(e)) {
            self.pbap = None;
            r = macbt::rfcomm(address, profiles::PBAP_SERVER).and_then(|ch| {
                let mut c = Client::new(ch);
                c.connect(&profiles::PBAP)?;
                let r = pull(&mut c);
                self.pbap = Some(c);
                r
            });
        }
        match r {
            Ok((pb, cch)) => {
                self.link.contacts = true;
                if let Some(p) = self.link.phone.as_mut() {
                    p.connected = true;
                }
                self.send(Event::Contacts(vcard::contacts(&pb)));
                if let Some(cch) = cch {
                    self.send(Event::Recents(vcard::calls(&cch)));
                }
                self.send(Event::Link(self.link.clone()));
            }
            Err(e) => self.error(format!("Contacts: {e}")),
        }
    }
}

/// Opens a message session with the phone at `address`, in its message folders.
fn connect_messages(address: &str) -> Result<Client<Rfcomm>, String> {
    let mut c = Client::new(macbt::rfcomm(address, profiles::MAP_SERVER)?);
    profiles::open_messages(&mut c)?;
    Ok(c)
}

/// Whether an error means the session has ended.
fn gone(e: &str) -> bool {
    e == crate::goep::GONE || e == crate::goep::SILENT
}

/// A notification for a new text, through Notification Center.
fn notify(title: &str, body: &str) {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let script = format!("display notification {} with title {}", quote(body), quote(title));
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}
