//! IMAP for reading and SMTP for sending, for Gmail (with an app password) and other
//! providers. It answers the same questions as the JMAP client so the worker treats every
//! account alike.
//!
//! Message ids are `uid|mailbox`: IMAP UIDs are unique only within a mailbox. Conversations
//! group by the root of each message's References (else In-Reply-To, else its own
//! Message-ID) among the messages fetched from the mailbox shown.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;
use imap_proto::NameAttribute;
use lettre::message::{Mailbox as LMailbox, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use mail_parser::MessageParser;

use crate::accounts::ImapConfig;
use crate::jmap::{Addr, Body, Mailbox, Outgoing, Summary, uid as app_uid};

type Stream = rustls_connector::TlsStream<TcpStream>;
type Session = imap::Session<Stream>;

/// How long a server may take to answer before the connection counts as gone. Without it, one
/// that stops answering (a connection left half-open by sleep or a network change) would hold
/// the worker, and every send behind it, for good.
const TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
pub type Result<T> = std::result::Result<T, String>;

/// Just the headers the list uses (Gmail's full headers carry kilobytes of signatures), and
/// the start of the text for the preview.
const LIST_FETCH: &str = "(UID FLAGS INTERNALDATE \
    BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES CONTENT-TYPE)] \
    BODY.PEEK[TEXT]<0.2048>)";

pub struct Imap {
    cfg: ImapConfig,
    session: Option<Session>,
    selected: Option<String>,
    can_move: bool,
    /// UIDPLUS: expunge only the messages a move flagged, not every `\Deleted` one.
    can_uid_expunge: bool,
}

/// What threading needs from a message beyond its summary.
struct Threading {
    root: String,
}

impl Imap {
    pub fn connect(cfg: ImapConfig) -> Result<Imap> {
        let mut imap = Imap { cfg, session: None, selected: None, can_move: false, can_uid_expunge: false };
        imap.session()?;
        Ok(imap)
    }

    /// STATUS of each mailbox (with HIGHESTMODSEQ where the server has CONDSTORE, so flag
    /// changes made elsewhere show too), joined into one comparable string.
    pub fn fingerprint(&mut self, mailboxes: &[String]) -> Result<String> {
        let mut out = String::new();
        for name in mailboxes {
            let full = self.with(|me| me.session.as_mut().unwrap().status(name, "(MESSAGES UIDNEXT UNSEEN HIGHESTMODSEQ)"));
            let st = match full {
                Ok(st) => st,
                Err(_) => self.with(|me| me.session.as_mut().unwrap().status(name, "(MESSAGES UIDNEXT UNSEEN)"))?,
            };
            out.push_str(&format!("{name}:{}:{:?}:{:?}:{:?};", st.exists, st.uid_next, st.unseen, st.highest_mod_seq));
        }
        Ok(out)
    }

    pub fn username(&self) -> &str {
        &self.cfg.email
    }

    fn open(&self) -> Result<Session> {
        let client =
            connect(&self.cfg.imap_host, self.cfg.imap_port).map_err(|e| format!("Couldn't reach {}: {e}", self.cfg.imap_host))?;
        client.login(&self.cfg.login, &self.cfg.password).map_err(|(e, _)| match e {
            imap::Error::No(_) | imap::Error::Bad(_) if self.cfg.gmail => {
                "Gmail rejected the app password. Check it, and that IMAP is on for the account.".to_owned()
            }
            e => format!("{} rejected the sign-in: {e}", self.cfg.imap_host),
        })
    }

    fn session(&mut self) -> Result<&mut Session> {
        if self.session.is_none() {
            let mut s = self.open()?;
            let caps = s.capabilities().ok();
            self.can_move = caps.as_ref().is_some_and(|c| c.has_str("MOVE"));
            self.can_uid_expunge = caps.as_ref().is_some_and(|c| c.has_str("UIDPLUS"));
            self.session = Some(s);
            self.selected = None;
        }
        Ok(self.session.as_mut().unwrap())
    }

    /// Runs `f`, reconnecting once if the connection dropped (servers close idle ones).
    fn with<T>(&mut self, f: impl FnMut(&mut Imap) -> imap::Result<T>) -> Result<T> {
        self.run(true, f)
    }

    /// Runs `f` once: for commands that may have taken effect before the connection dropped,
    /// where running them again would add a second copy.
    fn once<T>(&mut self, f: impl FnMut(&mut Imap) -> imap::Result<T>) -> Result<T> {
        self.run(false, f)
    }

    fn run<T>(&mut self, retry: bool, mut f: impl FnMut(&mut Imap) -> imap::Result<T>) -> Result<T> {
        self.session()?;
        match f(self) {
            Ok(v) => Ok(v),
            Err(e @ (imap::Error::Io(_) | imap::Error::ConnectionLost)) => {
                self.session = None;
                if !retry {
                    return Err(e.to_string());
                }
                self.session()?;
                f(self).map_err(|e| e.to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn select(&mut self, mailbox: &str) -> imap::Result<u32> {
        let s = self.session.as_mut().expect("connected");
        let mb = s.select(mailbox)?;
        self.selected = Some(mailbox.to_owned());
        Ok(mb.exists)
    }

    pub fn mailboxes(&mut self) -> Result<Vec<Mailbox>> {
        let names: Vec<(String, Option<&'static str>, Option<String>)> = self.with(|me| {
            let s = me.session.as_mut().unwrap();
            let list = s.list(None, Some("*"))?;
            Ok(list
                .iter()
                .filter(|n| !n.attributes().contains(&NameAttribute::NoSelect))
                .map(|n| {
                    let role = if n.name().eq_ignore_ascii_case("INBOX") { Some("inbox") } else { role(n.attributes()) };
                    (n.name().to_owned(), role, n.delimiter().map(str::to_owned))
                })
                .collect())
        })?;
        let mut out = Vec::new();
        // Gmail lists both \All and no \Archive; All Mail is where archiving goes there.
        let has_archive = names.iter().any(|(_, r, _)| *r == Some("archive"));
        for (name, role, delim) in &names {
            let role = match role {
                Some("all") if !has_archive => Some("archive"),
                Some("all") => None,
                r => *r,
            };
            // Counting Gmail's All Mail is slow, and its unread count (every unread message,
            // inbox included) isn't one the sidebar shows.
            let status = if self.cfg.gmail && role == Some("archive") && !has_archive {
                imap::types::Mailbox::default()
            } else {
                self.with(|me| me.session.as_mut().unwrap().status(name, "(MESSAGES UNSEEN)"))?
            };
            let (parent, short) = match delim.as_deref().and_then(|d| name.rsplit_once(d)) {
                Some((parent, short)) if names.iter().any(|(n, _, _)| n == parent) => (Some(parent.to_owned()), short),
                Some((_, short)) => (None, short),
                None => (None, name.as_str()),
            };
            out.push(Mailbox {
                id: name.clone(),
                name: if role == Some("inbox") { "Inbox".into() } else { short.to_owned() },
                role: role.map(str::to_owned),
                parent_id: parent,
                sort_order: 0,
                unread: status.unseen.unwrap_or(0),
                total: status.exists,
            });
        }
        Ok(out)
    }

    /// The UIDs of the newest `limit` messages in `mailbox` (matching `search`), oldest first.
    fn newest_uids(&mut self, mailbox: &str, search: &str, limit: usize) -> Result<Vec<u32>> {
        let search = search_criteria(search, self.cfg.gmail);
        let newest = u32::try_from(limit).unwrap_or(u32::MAX).saturating_sub(1);
        self.with(|me| {
            let exists = me.select(mailbox)?;
            if exists == 0 {
                return Ok(Vec::new());
            }
            let s = me.session.as_mut().unwrap();
            let mut uids: Vec<u32> = match &search {
                Some(criteria) => s.uid_search(criteria)?.into_iter().collect(),
                // A sequence range, then its UIDs.
                None => s.fetch(format!("{}:*", exists.saturating_sub(newest).max(1)), "UID")?.iter().filter_map(|f| f.uid).collect(),
            };
            uids.sort_unstable();
            let skip = uids.len().saturating_sub(limit);
            Ok(uids.split_off(skip))
        })
    }

    fn fetch_summaries(&mut self, mailbox: &str, uids: &[u32]) -> Result<Vec<(Summary, Threading)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let account = self.cfg.email.clone();
        let mailbox = mailbox.to_owned();
        self.with(|me| {
            if me.selected.as_deref() != Some(&mailbox) {
                me.select(&mailbox)?;
            }
            let fetches = me.session.as_mut().unwrap().uid_fetch(&set, LIST_FETCH)?;
            Ok(fetches.iter().filter_map(|f| summary(f, &account, &mailbox)).collect())
        })
    }

    pub fn messages(&mut self, mailbox: &str, search: &str, limit: usize) -> Result<Vec<Summary>> {
        let uids = self.newest_uids(mailbox, search, limit)?;
        let mut list: Vec<Summary> = self.fetch_summaries(mailbox, &uids)?.into_iter().map(|(m, _)| m).collect();
        list.sort_by(|a, b| b.received_at.cmp(&a.received_at));
        Ok(list)
    }

    /// Like JMAP's: the newest message of each conversation (in list order) and every
    /// message of them, here only among the messages fetched from `mailbox`.
    pub fn conversations(&mut self, mailbox: &str, search: &str, limit: usize) -> Result<(Vec<String>, Vec<Summary>)> {
        let uids = self.newest_uids(mailbox, search, limit)?;
        let mut all = self.fetch_summaries(mailbox, &uids)?;
        all.sort_by(|a, b| b.0.received_at.cmp(&a.0.received_at));
        let mut newest: Vec<String> = Vec::new();
        let mut seen_roots: HashMap<String, ()> = HashMap::new();
        for (m, t) in &mut all {
            m.thread_id = t.root.clone();
            if seen_roots.insert(t.root.clone(), ()).is_none() {
                newest.push(m.id.clone());
            }
        }
        Ok((newest, all.into_iter().map(|(m, _)| m).collect()))
    }

    fn raw(&mut self, id: &str) -> Result<Vec<u8>> {
        let (uid, mailbox) = split_id(id)?;
        self.with(|me| {
            if me.selected.as_deref() != Some(mailbox) {
                me.select(mailbox)?;
            }
            let fetches = me.session.as_mut().unwrap().uid_fetch(uid.to_string(), "BODY.PEEK[]")?;
            Ok(fetches.iter().next().and_then(|f| f.body()).map(<[u8]>::to_vec).unwrap_or_default())
        })
    }

    /// The bytes of attachment `part` (its position among the message's files).
    pub fn attachment(&mut self, id: &str, part: &str) -> Result<Vec<u8>> {
        let raw = self.raw(id)?;
        let msg = MessageParser::default().parse(&raw).ok_or("Couldn't read this message")?;
        let n: usize = part.parse().map_err(|_| format!("Bad attachment {part}"))?;
        let file = msg.attachments().filter(|p| is_file(p)).nth(n).ok_or("The attachment isn't there any more")?;
        Ok(file.decoded().into_owned())
    }

    pub fn body(&mut self, id: &str) -> Result<Body> {
        let raw = self.raw(id)?;
        let msg = MessageParser::default().parse(&raw).ok_or("Couldn't read this message")?;
        let files: Vec<crate::jmap::FileInfo> = msg
            .attachments()
            .filter(|p| is_file(p))
            .enumerate()
            .map(|(i, p)| crate::jmap::FileInfo {
                name: p.attachment_name().unwrap_or("attachment").to_owned(),
                mime: p.content_type().map_or("application/octet-stream".into(), |ct| {
                    format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream"))
                }),
                size: p.decoded().len() as u64,
                part: i.to_string(),
            })
            .collect();
        let text: Vec<String> = msg.text_bodies().map(|t| t.into_owned()).collect();
        let html: Vec<String> = msg.html_bodies().map(|h| h.into_owned()).collect();
        // Inline images become data URLs, so nothing else needs fetching from the server.
        let cids: Vec<(String, String)> = msg
            .attachments()
            .filter_map(|part| {
                let cid = part.content_id()?.trim_matches(['<', '>']).to_owned();
                let ct = part.content_type()?;
                let mime = format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream"));
                let b64 = base64::engine::general_purpose::STANDARD.encode(part.decoded());
                Some((cid, format!("data:{mime};base64,{b64}")))
            })
            .collect();
        let resolve = |id: &str| cids.iter().find(|(c, _)| c == id.trim_matches(['<', '>'])).map(|(_, s)| s.clone());
        let html_src = (!html.is_empty()).then(|| crate::html::resolve_cids(&html.join("\n"), &resolve));
        let html_doc = html_src.as_deref().map(|h| crate::html::parse(h, &|_| None));
        let plain = if text.is_empty() {
            crate::html::to_text(&html.join("\n"))
        } else {
            text.join("\n\n")
        };
        let list = |v: Option<mail_parser::TextList>| v.map_or_else(Vec::new, |l| l.iter().map(|s| format!("<{s}>")).collect());
        Ok(Body {
            text: plain.trim_end().to_owned(),
            html: html_doc,
            html_src,
            files,
            inline: Some(Vec::new()),
            cc: addrs(msg.cc()),
            reply_to: addrs(msg.reply_to()),
            message_id: msg.message_id().map(|m| vec![format!("<{m}>")]).unwrap_or_default(),
            references: list(msg.references()),
            unsubscribe: {
                let header = |name: &str| {
                    msg.headers().iter().find(|h| h.name().as_str().eq_ignore_ascii_case(name)).map(|h| {
                        String::from_utf8_lossy(h.raw_value()).split_whitespace().collect::<Vec<_>>().join(" ")
                    })
                };
                header("List-Unsubscribe").and_then(|u| {
                    crate::privacy::Unsubscribe::parse(&u, header("List-Unsubscribe-Post").as_deref(), header("List-Id").as_deref())
                })
            },
        })
    }

    /// Groups ids by mailbox and runs `f` with each mailbox selected and its UID set.
    /// Runs `f` on each mailbox's UID set; `retry` when running it twice does no harm.
    fn per_mailbox(
        &mut self,
        ids: &[String],
        retry: bool,
        mut f: impl FnMut(&mut Session, &str, &str) -> imap::Result<()>,
    ) -> Result<()> {
        let mut groups: HashMap<String, Vec<u32>> = HashMap::new();
        for id in ids {
            let (uid, mailbox) = split_id(id)?;
            groups.entry(mailbox.to_owned()).or_default().push(uid);
        }
        for (mailbox, uids) in groups {
            let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            self.run(retry, |me| {
                if me.selected.as_deref() != Some(&mailbox) {
                    me.select(&mailbox)?;
                }
                f(me.session.as_mut().unwrap(), &mailbox, &set)
            })?;
        }
        Ok(())
    }

    pub fn set_keyword(&mut self, ids: &[String], keyword: &str, on: bool) -> Result<()> {
        let flag = match keyword {
            "$seen" => "\\Seen",
            "$flagged" => "\\Flagged",
            other => return Err(format!("Can't set {other} over IMAP")),
        };
        let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        self.per_mailbox(ids, true, |s, _, set| s.uid_store(set, format!("{op} ({flag})")).map(drop))
    }

    pub fn move_to(&mut self, ids: &[String], _from: &str, to: &str) -> Result<()> {
        let (can_move, can_uid_expunge) = (self.can_move, self.can_uid_expunge);
        // A copy made before the connection dropped would be made again.
        self.per_mailbox(ids, can_move, |s, _, set| {
            if can_move {
                s.uid_mv(set, to)
            } else {
                s.uid_copy(set, to)?;
                s.uid_store(set, "+FLAGS.SILENT (\\Deleted)")?;
                if can_uid_expunge { s.uid_expunge(set).map(drop) } else { s.expunge().map(drop) }
            }
        })
    }

    /// Sends `mail`, then files a copy in `sent`. Ok once it's sent: a copy that couldn't be
    /// filed comes back as a note, since sending again would send it twice.
    pub fn send(&mut self, mail: &Outgoing, sent: Option<&str>) -> Result<Option<String>> {
        let from = LMailbox::new(mail.from_name.clone().or_else(|| self.cfg.name.clone()), self.cfg.email.parse().map_err(|e| format!("From: {e}"))?);
        // Message-ID and EHLO name come from the sender's domain, not this machine's hostname.
        let domain = self.cfg.email.rsplit_once('@').map_or("localhost", |(_, d)| d).to_owned();
        let mut b = lettre::Message::builder().from(from).subject(&mail.subject).message_id(Some(message_id(&domain)));
        let to_mailbox = |a: &Addr| -> Result<LMailbox> {
            Ok(LMailbox::new(a.name.clone(), a.email.parse().map_err(|e| format!("{}: {e}", a.email))?))
        };
        for a in &mail.to {
            b = b.to(to_mailbox(a)?);
        }
        for a in &mail.cc {
            b = b.cc(to_mailbox(a)?);
        }
        if let Some(id) = mail.in_reply_to.first() {
            b = b.in_reply_to(id.clone());
        }
        if !mail.references.is_empty() {
            b = b.references(mail.references.join(" "));
        }
        let message = if mail.html.is_none() && mail.attachments.is_empty() {
            b.header(ContentType::TEXT_PLAIN).body(mail.body.clone())
        } else {
            b.multipart(mime_parts(mail)?)
        }
        .map_err(|e| e.to_string())?;
        let creds = Credentials::new(self.cfg.login.clone(), self.cfg.password.clone());
        let transport = if self.cfg.smtp_starttls {
            SmtpTransport::starttls_relay(&self.cfg.smtp_host)
        } else {
            SmtpTransport::relay(&self.cfg.smtp_host)
        }
        .map_err(|e| e.to_string())?
        .port(self.cfg.smtp_port)
        .credentials(creds)
        .hello_name(lettre::transport::smtp::extension::ClientId::Domain(domain))
        .timeout(Some(std::time::Duration::from_secs(30)))
        .build();
        transport.send(&message).map_err(|e| format!("{}: {e}", self.cfg.smtp_host))?;
        if self.cfg.save_sent
            && let Some(sent) = sent
        {
            let raw = message.formatted();
            let filed = self.once(|me| {
                me.session.as_mut().unwrap().append(sent, &raw).flag(imap::types::Flag::Seen).finish().map(drop)
            });
            if let Err(e) = filed {
                return Ok(Some(format!("Sent, but a copy couldn't be saved to {sent}: {e}")));
            }
        }
        Ok(None)
    }
}

/// SEARCH criteria for what was typed in the search field; None when nothing was.
fn search_criteria(typed: &str, gmail: bool) -> Option<String> {
    // Quotes and backslashes would end the quoted string, and a line break would start a
    // command of its own.
    let query = typed.replace(|c: char| c == '"' || c == '\\' || c.is_control(), " ").trim().to_owned();
    if query.is_empty() {
        return None;
    }
    let charset = if query.is_ascii() { "" } else { "CHARSET UTF-8 " };
    let key = if gmail { "X-GM-RAW" } else { "TEXT" };
    Some(format!("{charset}{key} \"{query}\""))
}

/// A TLS connection to an IMAP server, from the start on 993 and after STARTTLS otherwise,
/// with reads and writes that give up after [`TIMEOUT`].
fn connect(host: &str, port: u16) -> Result<imap::Client<Stream>> {
    let mut last = format!("no address for {host}");
    let mut tcp = None;
    for addr in (host, port).to_socket_addrs().map_err(|e| e.to_string())? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(t) => {
                tcp = Some(t);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let tcp = tcp.ok_or(last)?;
    tcp.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    let implicit = port == 993;
    if !implicit {
        starttls(&tcp)?;
    }
    let tls = rustls_connector::RustlsConnector::new_with_native_certs()
        .map_err(|e| e.to_string())?
        .connect(host, tcp)
        .map_err(|e| e.to_string())?;
    let mut client = imap::Client::new(tls);
    if implicit {
        client.read_greeting().map_err(|e| e.to_string())?;
    } else {
        client.greeting_read = true;
    }
    Ok(client)
}

/// Reads the greeting and asks for STARTTLS over a plain connection.
fn starttls(tcp: &TcpStream) -> Result<()> {
    let mut reader = BufReader::new(tcp);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    (&*tcp).write_all(b"a0 STARTTLS\r\n").map_err(|e| e.to_string())?;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("the server closed the connection".into());
        }
        if let Some(status) = line.strip_prefix("a0 ") {
            return if status.starts_with("OK") { Ok(()) } else { Err("the server doesn't offer STARTTLS".into()) };
        }
    }
}

/// `<random@domain>`, without the machine's hostname lettre would use.
fn message_id(domain: &str) -> String {
    format!("<{:032x}@{domain}>", fastrand::u128(..))
}

/// The MIME tree for a message with HTML or attachments: the text (with the HTML and its inline
/// images as an alternative), then the files.
fn mime_parts(mail: &Outgoing) -> Result<lettre::message::MultiPart> {
    use lettre::message::{Attachment, MultiPart, SinglePart};
    let ct = |m: &str| ContentType::parse(m).unwrap_or(ContentType::TEXT_PLAIN);
    let text = SinglePart::plain(mail.body.clone());
    let body = match &mail.html {
        Some(html) => {
            let mut related = MultiPart::related().singlepart(SinglePart::html(html.clone()));
            for a in mail.attachments.iter().filter(|a| a.cid.is_some()) {
                let cid = a.cid.clone().unwrap_or_default();
                related = related.singlepart(Attachment::new_inline(cid).body(a.bytes.as_ref().clone(), ct(&a.mime)));
            }
            MultiPart::alternative().singlepart(text).multipart(related)
        }
        None => MultiPart::mixed().singlepart(text),
    };
    let files: Vec<_> = mail.attachments.iter().filter(|a| a.cid.is_none()).collect();
    if files.is_empty() {
        return Ok(body);
    }
    let mut mixed = MultiPart::mixed().multipart(body);
    for a in files {
        mixed = mixed.singlepart(Attachment::new(a.name.clone()).body(a.bytes.as_ref().clone(), ct(&a.mime)));
    }
    Ok(mixed)
}

fn role(attrs: &[NameAttribute]) -> Option<&'static str> {
    attrs.iter().find_map(|a| match a {
        NameAttribute::Sent => Some("sent"),
        NameAttribute::Drafts => Some("drafts"),
        NameAttribute::Trash => Some("trash"),
        NameAttribute::Junk => Some("junk"),
        NameAttribute::Archive => Some("archive"),
        NameAttribute::All => Some("all"),
        _ => None,
    })
}

/// An attachment shown as a file, not an image placed in the HTML.
fn is_file(p: &mail_parser::MessagePart) -> bool {
    p.content_id().is_none() || p.is_attachment()
}

fn split_id(id: &str) -> Result<(u32, &str)> {
    let (uid, mailbox) = id.split_once('|').ok_or_else(|| format!("Bad message id {id}"))?;
    Ok((uid.parse().map_err(|_| format!("Bad message id {id}"))?, mailbox))
}

fn addrs(list: Option<mail_parser::AddressList>) -> Vec<Addr> {
    list.map_or_else(Vec::new, |l| {
        l.mailboxes()
            .filter_map(|m| Some(Addr { name: m.name().map(str::to_owned), email: m.address()?.to_owned() }))
            .collect()
    })
}

fn summary(f: &imap::types::Fetch, account: &str, mailbox: &str) -> Option<(Summary, Threading)> {
    let uid = f.uid?;
    let mut raw = f.header().unwrap_or_default().to_vec();
    raw.extend_from_slice(f.text().unwrap_or_default());
    let msg = MessageParser::default().parse(&raw)?;
    let id = format!("{uid}|{mailbox}");
    let flags = f.flags();
    let received = f
        .internal_date()
        .map(|d| d.with_timezone(&chrono::Utc))
        .or_else(|| msg.date().and_then(|d| chrono::DateTime::from_timestamp(d.to_timestamp(), 0)))
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    let message_id = msg.message_id().map(str::to_owned);
    let root = msg
        .references()
        .and_then(|r| r.first().map(str::to_owned))
        .or_else(|| msg.in_reply_to().and_then(|r| r.first().map(str::to_owned)))
        .or(message_id)
        .unwrap_or_else(|| id.clone());
    let preview: String = msg
        .body_preview(240)
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let multipart_mixed = msg.root_part().content_type().is_some_and(|c| {
        c.ctype().eq_ignore_ascii_case("multipart") && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
    });
    let summary = Summary {
        uid: app_uid(account, &id),
        thread_key: app_uid(account, &id),
        thread_id: root.clone(),
        id,
        account: account.to_owned(),
        from: addrs(msg.from()),
        to: addrs(msg.to()),
        subject: msg.subject().unwrap_or_default().to_owned(),
        preview,
        received_at: received,
        seen: flags.contains(&imap::types::Flag::Seen),
        flagged: flags.contains(&imap::types::Flag::Flagged),
        has_attachment: multipart_mixed,
        mailbox_ids: vec![mailbox.to_owned()],
    };
    Some((summary, Threading { root }))
}

#[cfg(test)]
mod mime_tests {
    use super::*;
    use crate::attach::Outgoing as Att;
    use std::sync::Arc;

    #[test]
    fn inline_images_go_with_the_html_and_files_attach() {
        let mail = Outgoing {
            to: vec![Addr { name: None, email: "bob@x.com".into() }],
            body: "See Image 1".into(),
            html: Some("<div>See <img src=\"cid:img1@x\"></div>".into()),
            attachments: vec![
                Att { name: "image-1.png".into(), mime: "image/png".into(), bytes: Arc::new(vec![137, 80, 78, 71]), cid: Some("img1@x".into()) },
                Att { name: "report.pdf".into(), mime: "application/pdf".into(), bytes: Arc::new(b"%PDF".to_vec()), cid: None },
            ],
            ..Outgoing::default()
        };
        let m = lettre::Message::builder()
            .from("me@x.com".parse().unwrap())
            .to("bob@x.com".parse().unwrap())
            .subject("t")
            .multipart(mime_parts(&mail).unwrap())
            .unwrap();
        let raw = String::from_utf8_lossy(&m.formatted()).into_owned();
        for want in ["multipart/mixed", "multipart/alternative", "multipart/related", "text/plain", "text/html",
            "Content-ID: <img1@x>", "filename=\"report.pdf\"", "application/pdf"] {
            assert!(raw.contains(want), "missing {want}:\n{raw}");
        }
        assert!(raw.find("multipart/related").unwrap() < raw.find("report.pdf").unwrap(), "files come after the body");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_searches_stay_one_command() {
        assert_eq!(search_criteria("  ", false), None);
        assert_eq!(search_criteria("foo\r\nA9 EXPUNGE", false).as_deref(), Some("TEXT \"foo  A9 EXPUNGE\""));
        assert_eq!(search_criteria("say \"hi\"", true).as_deref(), Some("X-GM-RAW \"say  hi\""));
        assert_eq!(search_criteria("caf\u{e9}", false).as_deref(), Some("CHARSET UTF-8 TEXT \"caf\u{e9}\""));
    }

    #[test]
    fn a_server_without_starttls_is_refused_not_used_in_the_clear() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            c.write_all(b"* OK ready\r\n").unwrap();
            let mut line = String::new();
            BufReader::new(&c).read_line(&mut line).unwrap();
            c.write_all(b"a0 BAD no TLS here\r\n").unwrap();
        });
        let err = connect("127.0.0.1", port).expect_err("refused");
        assert!(err.contains("STARTTLS"), "{err}");
    }

    /// Opens TLS to Gmail's servers without signing in: checks the TLS setup works.
    /// Run with `cargo test -- --ignored` (needs the network).
    #[test]
    #[ignore]
    fn tls_reaches_gmail() {
        let client = connect("imap.gmail.com", 993).expect("IMAP over TLS");
        drop(client);
        let smtp = lettre::SmtpTransport::relay("smtp.gmail.com").unwrap().port(465).build();
        assert!(smtp.test_connection().expect("SMTP over TLS"));
    }
}
