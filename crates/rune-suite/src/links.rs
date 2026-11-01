//! Links between the apps: `rune-contacts://contact/<uid>`, `rune-calendar://day/2026-11-01`,
//! and the standard `mailto:`, `tel:` and `sms:`.
//!
//! Each app's launcher declares its scheme (`MimeType=x-scheme-handler/rune-mail;`), so a link
//! works from anywhere: a browser, a terminal (`xdg-open`), another app. [`open`] starts the
//! app the link is for, or hands the link to it when it's running (see [`crate::instance`]).
//!
//! | Link | Opens |
//! | --- | --- |
//! | `mailto:a@b.c?subject=Hi` | Mail, a new message |
//! | `rune-mail://message/<Message-ID>` | Mail, that message |
//! | `rune-calendar://day/2026-11-01`, `rune-calendar://event/<uid>` | Calendar, that day or event |
//! | `rune-contacts://contact/<uid>` | Contacts, that card |
//! | `rune-contacts://lookup?email=a@b.c` (or `phone=`, and `name=`) | Contacts: whoever has it, or a new card with it |
//! | `rune-notes://note/<path in the notes folder>` | Notes, that note |
//! | `tel:+15550100`, `rune-phone://call/<number>` | Phone, calling it |
//! | `sms:+15550100`, `rune-phone://message/<number>` | Phone, the conversation |
//! | `rune-passwords://search?q=example.com` | Passwords, searching |

use std::fmt;

/// The apps a link can be for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum App {
    Mail,
    Calendar,
    Contacts,
    Notes,
    Phone,
    Passwords,
}

impl App {
    pub const ALL: [App; 6] = [App::Mail, App::Calendar, App::Contacts, App::Notes, App::Phone, App::Passwords];

    pub fn scheme(self) -> &'static str {
        match self {
            App::Mail => "rune-mail",
            App::Calendar => "rune-calendar",
            App::Contacts => "rune-contacts",
            App::Notes => "rune-notes",
            App::Phone => "rune-phone",
            App::Passwords => "rune-passwords",
        }
    }

    /// The program, which is also its scheme.
    pub fn binary(self) -> &'static str {
        self.scheme()
    }

    pub fn of_scheme(scheme: &str) -> Option<App> {
        App::ALL.into_iter().find(|a| a.scheme().eq_ignore_ascii_case(scheme))
    }
}

/// A parsed link: which app, what kind of thing (`contact`, `day`), its id, and the query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub app: App,
    pub kind: String,
    pub id: String,
    pub query: Vec<(String, String)>,
}

impl Link {
    pub fn new(app: App, kind: &str, id: &str) -> Link {
        Link { app, kind: kind.to_owned(), id: id.to_owned(), query: Vec::new() }
    }

    pub fn with(mut self, key: &str, value: &str) -> Link {
        if !value.is_empty() {
            self.query.push((key.to_owned(), value.to_owned()));
        }
        self
    }

    /// A query value.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// Reads `mailto:`, `tel:`, `sms:` and `rune-*://kind/id?query` links; None for anything
    /// else. The text comes from outside the app: what it names is still to be checked.
    pub fn parse(text: &str) -> Option<Link> {
        let text = text.trim();
        let (scheme, rest) = text.split_once(':')?;
        let scheme = scheme.to_ascii_lowercase();
        let (target, query) = rest.split_once('?').unwrap_or((rest, ""));
        let query: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let decode = |s: &str| url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default();
        let link = match scheme.as_str() {
            "mailto" => Link { app: App::Mail, kind: "compose".into(), id: String::new(), query: Vec::new() }
                .with("to", &decode(target.trim_start_matches("//")))
                .extend(query),
            "tel" | "callto" => Link::new(App::Phone, "call", &decode(target.trim_start_matches("//"))),
            "sms" => Link::new(App::Phone, "message", &decode(target.trim_start_matches("//"))).extend(query),
            s => {
                let app = App::of_scheme(s)?;
                let path = target.strip_prefix("//")?;
                let (kind, id) = path.split_once('/').unwrap_or((path, ""));
                Link { app, kind: kind.to_ascii_lowercase(), id: decode(id.trim_end_matches('/')), query }
            }
        };
        Some(link)
    }

    fn extend(mut self, more: Vec<(String, String)>) -> Link {
        self.query.extend(more);
        self
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace("%2F", "/").replace('+', "%20");
        write!(f, "{}://{}", self.app.scheme(), self.kind)?;
        if !self.id.is_empty() {
            write!(f, "/{}", enc(&self.id))?;
        }
        if !self.query.is_empty() {
            let q: String = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(&self.query).finish();
            write!(f, "?{q}")?;
        }
        Ok(())
    }
}

/// `mailto:` for a new message to these addresses.
pub fn compose(to: &[&str], subject: &str) -> Link {
    Link::new(App::Mail, "compose", "").with("to", &to.join(",")).with("subject", subject)
}

/// A message in Mail, by its Message-ID (without the angle brackets).
pub fn message(message_id: &str) -> Link {
    Link::new(App::Mail, "message", message_id.trim().trim_start_matches('<').trim_end_matches('>'))
}

/// A day in Calendar, as `YYYY-MM-DD`.
pub fn day(date: &str) -> Link {
    Link::new(App::Calendar, "day", date)
}

/// An event in Calendar, by its UID.
pub fn event(uid: &str) -> Link {
    Link::new(App::Calendar, "event", uid)
}

/// A card in Contacts, by its UID.
pub fn contact(uid: &str) -> Link {
    Link::new(App::Contacts, "contact", uid)
}

/// Whoever in Contacts has this address, or a new card with it (and the name, if known).
pub fn contact_for_email(email: &str, name: &str) -> Link {
    Link::new(App::Contacts, "lookup", "").with("email", email).with("name", name)
}

/// Whoever in Contacts has this number, or a new card with it.
pub fn contact_for_phone(number: &str, name: &str) -> Link {
    Link::new(App::Contacts, "lookup", "").with("phone", number).with("name", name)
}

/// A note, by its path inside the notes folder.
pub fn note(path: &str) -> Link {
    Link::new(App::Notes, "note", path)
}

/// Calling a number from Phone.
pub fn call(number: &str) -> Link {
    Link::new(App::Phone, "call", number)
}

/// The conversation with a number in Phone.
pub fn text_message(number: &str) -> Link {
    Link::new(App::Phone, "message", number)
}

/// Searching Passwords.
pub fn password_search(query: &str) -> Link {
    Link::new(App::Passwords, "search", "").with("q", query)
}

/// Opens the link in its app: starts it, or hands the link to it if it's running. The app is
/// looked for beside this program first (they're installed together), then on the PATH, then
/// the system is asked (`xdg-open`).
pub fn open(link: &Link) -> Result<(), String> {
    let url = link.to_string();
    let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join(link.app.binary()))).filter(|p| p.is_file());
    let program = beside.map(|p| p.into_os_string()).unwrap_or_else(|| link.app.binary().into());
    match spawn(&program, &url) {
        Ok(()) => Ok(()),
        Err(_) => spawn(std::ffi::OsStr::new("xdg-open"), &url).map_err(|e| format!("Couldn't open {}: {e}", link.app.binary())),
    }
}

/// Runs `program url` without waiting, reaping it when it ends.
fn spawn(program: &std::ffi::OsStr, url: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_standard_and_rune_links() {
        let m = Link::parse("mailto:ada@example.com?subject=Hello%20there&cc=b@example.com").unwrap();
        assert_eq!((m.app, m.kind.as_str()), (App::Mail, "compose"));
        assert_eq!((m.get("to"), m.get("subject"), m.get("cc")), (Some("ada@example.com"), Some("Hello there"), Some("b@example.com")));
        let t = Link::parse("tel:+1%20555%200100").unwrap();
        assert_eq!((t.app, t.kind.as_str(), t.id.as_str()), (App::Phone, "call", "+1 555 0100"));
        let c = Link::parse("rune-contacts://lookup?email=a%40b.c&name=Ada+L").unwrap();
        assert_eq!((c.app, c.kind.as_str(), c.get("email"), c.get("name")), (App::Contacts, "lookup", Some("a@b.c"), Some("Ada L")));
        assert_eq!(Link::parse("https://example.com"), None);
        assert_eq!(Link::parse("rune-mail:nothing"), None);
    }

    #[test]
    fn writes_links_that_read_back() {
        for link in [
            message("<abc+1@mail.example.com>"),
            day("2026-11-01"),
            contact("A-1"),
            contact_for_email("a@b.c", "Ada Lovelace"),
            note("Work/Plans & ideas.md"),
            call("+1 555 0100"),
            password_search("example.com"),
        ] {
            assert_eq!(Link::parse(&link.to_string()).as_ref(), Some(&link), "{link}");
        }
        assert_eq!(message("<abc@x>").id, "abc@x");
        assert_eq!(note("Work/Plan.md").to_string(), "rune-notes://note/Work/Plan.md");
    }
}
