//! Message Access and Phonebook Access spoken directly over OBEX (`goep`): what obexd does for
//! the app on Linux, for macOS, which has no obexd. Messages are named by the phone's handle.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use crate::goep::{self, Client, Stream};
use crate::obex::Listed;

pub type Result<T> = std::result::Result<T, String>;

/// The Message Access service's OBEX target.
pub const MAP: [u8; 16] = [0xbb, 0x58, 0x2b, 0x40, 0x42, 0x0c, 0x11, 0xdb, 0xb0, 0xde, 0x08, 0x00, 0x20, 0x0c, 0x9a, 0x66];
/// The Phonebook Access service's OBEX target.
pub const PBAP: [u8; 16] = [0x79, 0x61, 0x35, 0xf0, 0xf0, 0xc5, 0x11, 0xd8, 0x09, 0x66, 0x08, 0x00, 0x20, 0x0c, 0x9a, 0x66];

/// Their service class UUIDs, to find their RFCOMM channels.
pub const MAP_SERVER: u16 = 0x1132;
pub const PBAP_SERVER: u16 = 0x112F;

// Message Access application parameters.
const MAX_LIST_COUNT: u8 = 0x01;
const ATTACHMENT: u8 = 0x0A;
const SUBJECT_LENGTH: u8 = 0x13;
const CHARSET: u8 = 0x14;
const STATUS_INDICATOR: u8 = 0x17;
const STATUS_VALUE: u8 = 0x18;
const UTF8: u8 = 1;

// Phonebook Access application parameters.
const FORMAT: u8 = 0x07;
const PB_MAX_LIST_COUNT: u8 = 0x04;
const VCARD30: u8 = 1;

/// The message ids the window sees: the handle, shaped like obexd's object paths (whose last
/// part the window matches messages by).
pub fn id(handle: &str) -> String {
    format!("message{handle}")
}

pub fn handle(id: &str) -> &str {
    let last = id.rsplit('/').next().unwrap_or(id);
    last.strip_prefix("message").unwrap_or(last)
}

/// Connects a Message Access session and goes into the message folders.
pub fn open_messages<S: Stream>(c: &mut Client<S>) -> Result<()> {
    c.connect(&MAP)?;
    c.set_path("telecom")?;
    c.set_path("msg")
}

/// The newest `count` messages in `folder` ("inbox", "sent"…).
pub fn list<S: Stream>(c: &mut Client<S>, folder: &str, count: u16) -> Result<Vec<Listed>> {
    let app = goep::params(&[(MAX_LIST_COUNT, &count.to_be_bytes()), (SUBJECT_LENGTH, &[255])]);
    let xml = c.get(Some(folder), "x-bt/MAP-msg-listing", &app)?;
    let mut out = listing(&String::from_utf8_lossy(&xml), folder);
    out.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(out)
}

/// A message whole, as a bMessage.
pub fn get<S: Stream>(c: &mut Client<S>, id: &str) -> Result<crate::bmsg::Parsed> {
    let app = goep::params(&[(ATTACHMENT, &[0]), (CHARSET, &[UTF8])]);
    let b = c.get(Some(handle(id)), "x-bt/message", &app)?;
    Ok(crate::bmsg::parse(&String::from_utf8_lossy(&b)))
}

fn set_status<S: Stream>(c: &mut Client<S>, id: &str, indicator: u8, value: bool) -> Result<()> {
    let app = goep::params(&[(STATUS_INDICATOR, &[indicator]), (STATUS_VALUE, &[value as u8])]);
    // The body is a filler byte: the request is all in its parameters.
    c.put(handle(id), "x-bt/messageStatus", &app, b"0").map(drop)
}

pub fn set_read<S: Stream>(c: &mut Client<S>, id: &str, read: bool) -> Result<()> {
    set_status(c, id, 0, read)
}

pub fn delete<S: Stream>(c: &mut Client<S>, id: &str) -> Result<()> {
    set_status(c, id, 1, true)
}

/// Sends `text` to `number` as an SMS, through the phone's outbox.
pub fn send<S: Stream>(c: &mut Client<S>, number: &str, text: &str) -> Result<()> {
    let app = goep::params(&[(CHARSET, &[UTF8])]);
    c.put("outbox", "x-bt/message", &app, crate::bmsg::compose_sms(number, text).as_bytes()).map(drop)
}

/// One of the phone's phonebooks as vCard text: "pb" (contacts) or "cch" (call history), from a
/// session already connected to `PBAP`.
pub fn phonebook<S: Stream>(c: &mut Client<S>, book: &str) -> Result<String> {
    let app = goep::params(&[(FORMAT, &[VCARD30]), (PB_MAX_LIST_COUNT, &u16::MAX.to_be_bytes())]);
    let v = c.get(Some(&format!("telecom/{book}.vcf")), "x-bt/phonebook", &app)?;
    Ok(String::from_utf8_lossy(&v).into_owned())
}

/// The messages of a message listing (`MAP-msg-listing` XML).
pub fn listing(xml: &str, folder: &str) -> Vec<Listed> {
    let incoming = folder == "inbox";
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<msg") {
        rest = &rest[i + 4..];
        // `<msg` and then whitespace: not some other element starting the same way.
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let end = rest.find('>').unwrap_or(rest.len());
        let a = attributes(&rest[..end]);
        rest = &rest[end..];
        let get = |k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let Some(h) = Some(get("handle")).filter(|h| !h.is_empty()) else { continue };
        let (name, number) =
            if incoming { (get("sender_name"), get("sender_addressing")) } else { (get("recipient_name"), get("recipient_addressing")) };
        out.push(Listed {
            path: id(&h),
            folder: folder.to_owned(),
            name,
            number,
            preview: get("subject"),
            // `YYYYMMDDTHHMMSS`, sometimes with a UTC offset after it.
            at: get("datetime").chars().take(15).collect(),
            read: get("read") == "yes" || !incoming,
            incoming,
            mms: get("type") == "MMS",
        });
    }
    out
}

/// An element's attributes, their values unescaped.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().rsplit(char::is_whitespace).next().unwrap_or("").to_owned();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let body = &after[1..];
        let Some(close) = body.find(q) else { break };
        out.push((name, unescape(&body[..close])));
        rest = &body[close + 1..];
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|n| *n <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goep::tests::{Script, connected, reply};
    use crate::goep::{BODY, END_OF_BODY, NAME, bytes, headers, unicode};

    const LISTING: &str = r#"<?xml version="1.0"?>
<MAP-msg-listing version="1.0">
  <msg handle="0400000000000012" subject="Fish &amp; chips at 7?" datetime="20261031T120000+0100"
       sender_name="Ann" sender_addressing="+15550100" recipient_addressing="" type="SMS_GSM"
       size="20" reception_status="complete" read="no" priority="no" protected="no"/>
  <msg handle='0400000000000013' subject="&#x1F44D; ok" datetime="20261031T121500"
       sender_name="" sender_addressing="+15550101" type="MMS" read="yes"/>
</MAP-msg-listing>"#;

    #[test]
    fn reads_a_message_listing() {
        let l = listing(LISTING, "inbox");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].path, "message0400000000000012");
        assert_eq!((l[0].name.as_str(), l[0].number.as_str()), ("Ann", "+15550100"));
        assert_eq!(l[0].preview, "Fish & chips at 7?");
        assert_eq!(l[0].at, "20261031T120000");
        assert!(l[0].incoming && !l[0].read && !l[0].mms);
        assert_eq!(l[1].preview, "👍 ok");
        assert!(l[1].read && l[1].mms);
        let sent = listing(r#"<msg handle="9" recipient_name="Bo" recipient_addressing="+1555" sender_addressing="me" read="no"/>"#, "sent");
        assert_eq!((sent[0].name.as_str(), sent[0].number.as_str(), sent[0].read, sent[0].incoming), ("Bo", "+1555", true, false));
    }

    #[test]
    fn ids_round_trip_to_handles() {
        assert_eq!(handle(&id("04000012")), "04000012");
        assert_eq!(handle("/org/bluez/obex/client/session3/message77"), "77");
    }

    #[test]
    fn lists_marks_and_sends_through_the_session() {
        let ok = || reply(0xA0, &[], &[]);
        let s = Script::new(vec![
            connected(0x400),
            ok(),
            ok(),
            reply(0xA0, &[], &[bytes(END_OF_BODY, LISTING.as_bytes())]),
            ok(),
            reply(0xA0, &[], &[unicode(NAME, "0400000000000099")]),
        ]);
        let mut c = Client::new(s);
        open_messages(&mut c).unwrap();
        let l = list(&mut c, "inbox", 50).unwrap();
        assert_eq!(l.len(), 2);
        set_read(&mut c, &l[0].path, true).unwrap();
        send(&mut c, "+15550100", "On my way").unwrap();
        let sent = c.into_sent();
        let names: Vec<String> = sent[1..]
            .iter()
            .map(|p| {
                headers(&p[3 + if p[0] == 0x85 { 2 } else { 0 }..])
                    .unwrap()
                    .into_iter()
                    .find(|(h, _)| *h == NAME)
                    .map(|(_, v)| String::from_utf16_lossy(&v.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>()).trim_end_matches('\0').to_owned())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(names, ["telecom", "msg", "inbox", "0400000000000012", "outbox"]);
        let push = headers(&sent[5][3..]).unwrap();
        let body = &push.iter().find(|(h, _)| *h == END_OF_BODY || *h == BODY).unwrap().1;
        assert!(String::from_utf8_lossy(body).contains("TEL:+15550100"));
    }
}
