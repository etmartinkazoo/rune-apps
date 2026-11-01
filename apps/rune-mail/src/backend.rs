//! One account's server, whichever protocol it speaks: JMAP for Fastmail, IMAP and SMTP for
//! Gmail and the rest.

use crate::accounts::Saved;
use crate::imap::Imap;
use crate::jmap::{Body, Client, Downloads, Mailbox, Outgoing, Summary};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Fastmail,
    Gmail,
    Imap,
}

pub enum Backend {
    Jmap(Client),
    /// Boxed: the IMAP session is held inline and is several times the size of a JMAP client.
    Imap(Box<Imap>),
}

impl Backend {
    pub fn connect(saved: &Saved) -> Result<Backend> {
        if saved.locked {
            return Err("The system keyring wouldn't give this account's password. Unlock it and reopen Rune Mail.".into());
        }
        match &saved.imap {
            Some(cfg) => Imap::connect(cfg.clone()).map(|i| Backend::Imap(Box::new(i))),
            None => Client::connect(&saved.token).map(Backend::Jmap),
        }
    }

    pub fn username(&self) -> &str {
        match self {
            Backend::Jmap(c) => &c.username,
            Backend::Imap(i) => i.username(),
        }
    }

    pub fn kind(&self, saved: &Saved) -> Kind {
        match (self, &saved.imap) {
            (Backend::Jmap(_), _) => Kind::Fastmail,
            (Backend::Imap(_), Some(cfg)) if cfg.gmail => Kind::Gmail,
            (Backend::Imap(_), _) => Kind::Imap,
        }
    }

    /// Fetches images: account blobs over JMAP; IMAP inlines its own, so only the web.
    pub fn downloads(&self) -> Downloads {
        match self {
            Backend::Jmap(c) => c.downloads.clone(),
            Backend::Imap(_) => Downloads::web(),
        }
    }

    pub fn is_imap(&self) -> bool {
        matches!(self, Backend::Imap(_))
    }

    /// A short string that changes whenever mail in `mailboxes` does: JMAP's Email state for
    /// the whole account, IMAP STATUS of each mailbox. Equal strings mean nothing to reload.
    pub fn fingerprint(&mut self, mailboxes: &[String]) -> Result<String> {
        match self {
            Backend::Jmap(c) => c.email_state(),
            Backend::Imap(i) => i.fingerprint(mailboxes),
        }
    }

    pub fn mailboxes(&mut self) -> Result<Vec<Mailbox>> {
        match self {
            Backend::Jmap(c) => c.mailboxes(),
            Backend::Imap(i) => i.mailboxes(),
        }
    }

    pub fn messages(&mut self, mailbox: &str, search: &str, limit: usize) -> Result<Vec<Summary>> {
        match self {
            Backend::Jmap(c) => c.messages(mailbox, search, limit),
            Backend::Imap(i) => i.messages(mailbox, search, limit),
        }
    }

    pub fn conversations(&mut self, mailbox: &str, search: &str, limit: usize) -> Result<(Vec<String>, Vec<Summary>)> {
        match self {
            Backend::Jmap(c) => c.conversations(mailbox, search, limit),
            Backend::Imap(i) => i.conversations(mailbox, search, limit),
        }
    }

    /// An attachment's bytes: a blob over JMAP, a part of the message over IMAP.
    pub fn attachment(&mut self, id: &str, file: &crate::jmap::FileInfo) -> Result<Vec<u8>> {
        match self {
            Backend::Jmap(c) => c.downloads.file(&file.part, &file.mime),
            Backend::Imap(i) => i.attachment(id, &file.part),
        }
    }

    pub fn body(&mut self, id: &str) -> Result<Body> {
        match self {
            Backend::Jmap(c) => c.body(id),
            Backend::Imap(i) => i.body(id),
        }
    }

    pub fn set_keyword(&mut self, ids: &[String], keyword: &str, on: bool) -> Result<()> {
        match self {
            Backend::Jmap(c) => c.set_keyword(ids, keyword, on),
            Backend::Imap(i) => i.set_keyword(ids, keyword, on),
        }
    }

    pub fn move_to(&mut self, ids: &[String], from: &str, to: &str) -> Result<()> {
        match self {
            Backend::Jmap(c) => c.move_to(ids, from, to),
            Backend::Imap(i) => i.move_to(ids, from, to),
        }
    }

    /// Ok once the mail is on its way, with a note about anything after that which didn't
    /// work (such as filing a copy in Sent).
    pub fn send(&mut self, mail: &Outgoing, mailboxes: &[Mailbox]) -> Result<Option<String>> {
        let role = |r: &str| mailboxes.iter().find(|m| m.role.as_deref() == Some(r)).map(|m| m.id.clone());
        match self {
            Backend::Jmap(c) => match (role("drafts"), role("sent")) {
                (Some(drafts), Some(sent)) => c.send(mail, &drafts, &sent).map(|()| None),
                _ => Err("This account has no Drafts or Sent mailbox".to_owned()),
            },
            Backend::Imap(i) => i.send(mail, role("sent").as_deref()),
        }
    }
}
