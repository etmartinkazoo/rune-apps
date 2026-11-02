//! An OBEX client over a byte stream: what Message Access and Phonebook Access sessions speak.
//! On Linux obexd does this; on macOS there's nothing like it, so the app speaks OBEX itself
//! over an RFCOMM channel (see `macbt`). It knows nothing of Bluetooth: any `Stream` will do,
//! which the tests use to stand in for a phone.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::time::Duration;

pub type Result<T> = std::result::Result<T, String>;

/// The stream closed under the session.
pub const GONE: &str = "The phone went away";
/// The phone stopped replying mid-session.
pub const SILENT: &str = "The phone stopped answering";

/// Bytes in and out: an RFCOMM channel, or a test's script.
pub trait Stream {
    fn write(&mut self, data: &[u8]) -> Result<()>;
    /// Bytes that have come in, waiting up to `timeout` for some; none means none came.
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>>;
}

// Header ids. The top two bits say the kind: 00 text (UTF-16), 01 bytes, 10 one byte, 11 four.
pub const NAME: u8 = 0x01;
pub const TYPE: u8 = 0x42;
pub const TARGET: u8 = 0x46;
pub const BODY: u8 = 0x48;
pub const END_OF_BODY: u8 = 0x49;
pub const APP_PARAMS: u8 = 0x4C;
pub const CONNECTION_ID: u8 = 0xCB;

const CONNECT: u8 = 0x80;
const DISCONNECT: u8 = 0x81;
const PUT: u8 = 0x02;
const GET: u8 = 0x03;
const SETPATH: u8 = 0x85;
const FINAL: u8 = 0x80;

const CONTINUE: u8 = 0x90;
const SUCCESS: u8 = 0xA0;

/// The largest packet this end takes.
const OUR_MAX: u16 = 0xFFFE;

/// A text header: UTF-16, big-endian, ending in a null.
pub fn unicode(hi: u8, s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() * 2 + 2);
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_be_bytes());
    }
    v.extend_from_slice(&[0, 0]);
    bytes(hi, &v)
}

/// A byte-sequence header.
pub fn bytes(hi: u8, v: &[u8]) -> Vec<u8> {
    let n = (v.len() + 3) as u16;
    let mut out = vec![hi];
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(v);
    out
}

/// Application parameters, as the value of an `APP_PARAMS` header: tag, length, value.
pub fn params(items: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, v) in items {
        out.push(*tag);
        out.push(v.len() as u8);
        out.extend_from_slice(v);
    }
    out
}

/// A packet's headers (what follows its opcode and length, and any fixed fields).
pub fn headers(p: &[u8]) -> Result<Vec<(u8, Vec<u8>)>> {
    let bad = || "The phone sent a malformed OBEX packet".to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let hi = p[i];
        let n = match hi >> 6 {
            0 | 1 => {
                let n = u16::from_be_bytes([*p.get(i + 1).ok_or_else(bad)?, *p.get(i + 2).ok_or_else(bad)?]) as usize;
                if n < 3 {
                    return Err(bad());
                }
                out.push((hi, p.get(i + 3..i + n).ok_or_else(bad)?.to_vec()));
                n
            }
            2 => {
                out.push((hi, p.get(i + 1..i + 2).ok_or_else(bad)?.to_vec()));
                2
            }
            _ => {
                out.push((hi, p.get(i + 1..i + 5).ok_or_else(bad)?.to_vec()));
                5
            }
        };
        i += n;
    }
    Ok(out)
}

/// What a response code means for the person.
fn refused(code: u8) -> String {
    match code & !FINAL {
        // Unauthorized, Forbidden, Service Unavailable: Android answers these when access is off.
        0x41 | 0x43 | 0x53 => "The phone didn't allow it: on the phone, allow access in this computer's Bluetooth settings".into(),
        0x44 => "The phone doesn't have that".into(),
        0x4D | 0x4F => "The phone couldn't take it".into(),
        c => format!("The phone said no (OBEX 0x{:02X})", c | FINAL),
    }
}

/// A response: its code, and its headers.
#[derive(Debug)]
pub struct Response {
    pub code: u8,
    pub headers: Vec<(u8, Vec<u8>)>,
}

impl Response {
    #[cfg(test)]
    pub fn header(&self, hi: u8) -> Option<&[u8]> {
        self.headers.iter().find(|(h, _)| *h == hi).map(|(_, v)| v.as_slice())
    }
}

pub struct Client<S: Stream> {
    stream: S,
    buf: Vec<u8>,
    connection: Option<[u8; 4]>,
    /// The largest packet the phone takes.
    max: usize,
    /// How long a reply may take. The first one can wait on the person allowing access.
    pub timeout: Duration,
}

impl<S: Stream> Client<S> {
    pub fn new(stream: S) -> Self {
        Client { stream, buf: Vec::new(), connection: None, max: 255, timeout: Duration::from_secs(30) }
    }

    /// One whole packet off the stream.
    fn packet(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.buf.len() >= 3 {
                let n = u16::from_be_bytes([self.buf[1], self.buf[2]]) as usize;
                if n < 3 {
                    return Err("The phone sent a malformed OBEX packet".into());
                }
                if self.buf.len() >= n {
                    return Ok(self.buf.drain(..n).collect());
                }
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(SILENT.into());
            }
            let got = self.stream.read(left.min(Duration::from_millis(500)))?;
            self.buf.extend_from_slice(&got);
        }
    }

    /// Sends a request and reads its response. `fixed` is the fields some opcodes have before
    /// their headers; `skip` the same for the response.
    fn request(&mut self, op: u8, fixed: &[u8], headers: &[u8], skip: usize, timeout: Duration) -> Result<Response> {
        let n = 3 + fixed.len() + headers.len();
        let mut p = Vec::with_capacity(n);
        p.push(op);
        p.extend_from_slice(&(n as u16).to_be_bytes());
        p.extend_from_slice(fixed);
        p.extend_from_slice(headers);
        self.stream.write(&p)?;
        let r = self.packet(timeout)?;
        Ok(Response { code: r[0], headers: self::headers(r.get(3 + skip..).unwrap_or_default())? })
    }

    fn connection_header(&self) -> Vec<u8> {
        match self.connection {
            Some(id) => [&[CONNECTION_ID][..], &id].concat(),
            None => Vec::new(),
        }
    }

    /// Connects to the service named by `target` (its UUID's 16 bytes).
    pub fn connect(&mut self, target: &[u8]) -> Result<()> {
        // Version 1.0, no flags, the largest packet taken here.
        let fixed = [&[0x10, 0x00][..], &OUR_MAX.to_be_bytes()].concat();
        // Long enough for the phone to ask the person whether to allow it.
        let wait = self.timeout.max(Duration::from_secs(60));
        let n = 3 + fixed.len() + 19;
        let mut p = vec![CONNECT];
        p.extend_from_slice(&(n as u16).to_be_bytes());
        p.extend_from_slice(&fixed);
        p.extend_from_slice(&bytes(TARGET, target));
        self.stream.write(&p)?;
        let r = self.packet(wait)?;
        if r[0] != SUCCESS {
            return Err(refused(r[0]));
        }
        if r.len() < 7 {
            return Err("The phone sent a malformed OBEX packet".into());
        }
        self.max = (u16::from_be_bytes([r[5], r[6]]) as usize).max(255);
        let hs = headers(&r[7..])?;
        self.connection = hs.iter().find(|(h, _)| *h == CONNECTION_ID).and_then(|(_, v)| v.as_slice().try_into().ok());
        Ok(())
    }

    /// Fetches an object, all of it.
    pub fn get(&mut self, name: Option<&str>, kind: &str, app: &[u8]) -> Result<Vec<u8>> {
        let mut h = self.connection_header();
        if let Some(name) = name {
            h.extend(unicode(NAME, name));
        }
        h.extend(bytes(TYPE, &[kind.as_bytes(), &[0]].concat()));
        if !app.is_empty() {
            h.extend(bytes(APP_PARAMS, app));
        }
        let mut body = Vec::new();
        let mut r = self.request(GET | FINAL, &[], &h, 0, self.timeout)?;
        loop {
            for (hi, v) in &r.headers {
                if *hi == BODY || *hi == END_OF_BODY {
                    body.extend_from_slice(v);
                }
            }
            match r.code {
                CONTINUE => {
                    let h = self.connection_header();
                    r = self.request(GET | FINAL, &[], &h, 0, self.timeout)?;
                }
                SUCCESS => return Ok(body),
                c => return Err(refused(c)),
            }
        }
    }

    /// Sends an object, split across as many packets as the phone needs.
    pub fn put(&mut self, name: &str, kind: &str, app: &[u8], body: &[u8]) -> Result<Response> {
        let mut h = self.connection_header();
        h.extend(unicode(NAME, name));
        h.extend(bytes(TYPE, &[kind.as_bytes(), &[0]].concat()));
        if !app.is_empty() {
            h.extend(bytes(APP_PARAMS, app));
        }
        let mut rest = body;
        loop {
            // Room for the packet's own 3 bytes, the body header's 3, and the headers so far.
            let room = self.max.saturating_sub(6 + h.len()).max(1);
            let (chunk, more) = rest.split_at(rest.len().min(room));
            rest = more;
            let last = rest.is_empty();
            h.extend(bytes(if last { END_OF_BODY } else { BODY }, chunk));
            let r = self.request(if last { PUT | FINAL } else { PUT }, &[], &h, 0, self.timeout)?;
            match (last, r.code) {
                (true, SUCCESS) => return Ok(r),
                (false, CONTINUE) => h = Vec::new(),
                (_, c) => return Err(refused(c)),
            }
        }
    }

    /// Goes into the folder `name` (it isn't made if it isn't there).
    pub fn set_path(&mut self, name: &str) -> Result<()> {
        let mut h = self.connection_header();
        h.extend(unicode(NAME, name));
        let r = self.request(SETPATH, &[0x02, 0x00], &h, 0, self.timeout)?;
        if r.code == SUCCESS { Ok(()) } else { Err(refused(r.code)) }
    }

    /// Ends the session; the phone may already be gone, which is fine.
    pub fn disconnect(&mut self) {
        let h = self.connection_header();
        let _ = self.request(DISCONNECT, &[], &h, 0, Duration::from_secs(3));
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A phone that answers each packet with the next of its replies, cut into pieces as a
    /// channel would deliver them.
    #[derive(Default)]
    pub struct Script {
        pub replies: VecDeque<Vec<u8>>,
        pub sent: Vec<Vec<u8>>,
        pending: VecDeque<Vec<u8>>,
    }

    impl Script {
        pub fn new(replies: Vec<Vec<u8>>) -> Self {
            Script { replies: replies.into(), ..Default::default() }
        }
    }

    impl Stream for Script {
        fn write(&mut self, data: &[u8]) -> Result<()> {
            self.sent.push(data.to_vec());
            if let Some(r) = self.replies.pop_front() {
                for piece in r.chunks(7) {
                    self.pending.push_back(piece.to_vec());
                }
            }
            Ok(())
        }
        fn read(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
            self.pending.pop_front().ok_or_else(|| "nothing more from the phone".to_string())
        }
    }

    impl Client<Script> {
        pub fn into_sent(self) -> Vec<Vec<u8>> {
            self.stream.sent
        }
    }

    /// A response packet.
    pub fn reply(code: u8, fixed: &[u8], headers: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = fixed.iter().copied().chain(headers.iter().flatten().copied()).collect();
        let mut p = vec![code];
        p.extend_from_slice(&((body.len() + 3) as u16).to_be_bytes());
        p.extend(body);
        p
    }

    pub fn connected(max: u16) -> Vec<u8> {
        reply(SUCCESS, &[0x10, 0x00, (max >> 8) as u8, max as u8], &[vec![CONNECTION_ID, 0, 0, 0, 7]])
    }

    #[test]
    fn connects_and_gets_across_packets() {
        let s = Script::new(vec![
            connected(0x400),
            reply(CONTINUE, &[], &[bytes(BODY, b"BEGIN:VCARD\r\n")]),
            reply(SUCCESS, &[], &[bytes(END_OF_BODY, b"END:VCARD\r\n")]),
        ]);
        let mut c = Client::new(s);
        c.connect(&[1; 16]).unwrap();
        assert_eq!(c.max, 0x400);
        let got = c.get(Some("telecom/pb.vcf"), "x-bt/phonebook", &params(&[(0x07, &[1])])).unwrap();
        assert_eq!(got, b"BEGIN:VCARD\r\nEND:VCARD\r\n");
        let sent = &c.stream.sent;
        assert_eq!(&sent[0][..3], &[CONNECT, 0, 26]);
        assert_eq!(&sent[0][7..10], &[TARGET, 0, 19]);
        // Every request after connecting names the connection first.
        assert_eq!(&sent[1][3..8], &[CONNECTION_ID, 0, 0, 0, 7]);
        assert_eq!(sent[1][0], GET | FINAL);
        let hs = headers(&sent[1][3..]).unwrap();
        assert_eq!(hs[1], (NAME, [&b"\0t\0e\0l\0e\0c\0o\0m\0/\0p\0b\0.\0v\0c\0f"[..], &[0, 0]].concat()));
        assert_eq!(hs[2], (TYPE, b"x-bt/phonebook\0".to_vec()));
        assert_eq!(hs[3], (APP_PARAMS, vec![0x07, 1, 1]));
        assert_eq!(sent[2], vec![GET | FINAL, 0, 8, CONNECTION_ID, 0, 0, 0, 7], "a continuation only names the connection");
    }

    #[test]
    fn puts_in_pieces_no_bigger_than_the_phone_takes() {
        let body = vec![b'x'; 600];
        let s = Script::new(vec![connected(255), reply(CONTINUE, &[], &[]), reply(CONTINUE, &[], &[]), reply(SUCCESS, &[], &[unicode(NAME, "42")])]);
        let mut c = Client::new(s);
        c.connect(&[1; 16]).unwrap();
        let r = c.put("outbox", "x-bt/message", &[], &body).unwrap();
        assert_eq!(r.header(NAME).unwrap(), &[0, b'4', 0, b'2', 0, 0]);
        let puts = &c.stream.sent[1..];
        assert_eq!(puts.len(), 3);
        assert!(puts.iter().all(|p| p.len() <= 255));
        assert_eq!(puts.iter().map(|p| p[0]).collect::<Vec<_>>(), [PUT, PUT, PUT | FINAL]);
        let carried: usize = puts.iter().flat_map(|p| headers(&p[3..]).unwrap()).filter(|(h, _)| *h == BODY || *h == END_OF_BODY).map(|(_, v)| v.len()).sum();
        assert_eq!(carried, 600);
    }

    #[test]
    fn a_refusal_says_where_to_allow_it() {
        let mut c = Client::new(Script::new(vec![reply(0xD3, &[], &[])]));
        let e = c.connect(&[1; 16]).unwrap_err();
        assert!(e.contains("allow access"), "{e}");
    }

    #[test]
    fn set_path_does_not_make_folders() {
        let mut c = Client::new(Script::new(vec![connected(0x400), reply(SUCCESS, &[], &[])]));
        c.connect(&[1; 16]).unwrap();
        c.set_path("telecom").unwrap();
        let p = &c.stream.sent[1];
        assert_eq!((p[0], p[3], p[4]), (SETPATH, 0x02, 0x00));
    }
}
