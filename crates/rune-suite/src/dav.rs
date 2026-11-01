//! WebDAV, as CalDAV and CardDAV use it, signed in with an app password.
//!
//! [`Dav`] sends the requests; [`Dav::home`] finds an account's calendar or address-book home
//! from a starting address (RFC 6764: the current user's principal, then its home set, trying
//! `/.well-known/` when the address itself doesn't answer). The CardDAV calls Rune Contacts
//! uses are here too: address books, their cards with ETags, and saving and deleting a card.

use base64::Engine;

pub type Result<T> = std::result::Result<T, String>;

pub const DAV: &str = "DAV:";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
pub const APPLE: &str = "http://apple.com/ns/ical/";
const CALSERVER: &str = "http://calendarserver.org/ns/";

/// A DAV server's answer.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub etag: Option<String>,
    pub body: String,
}

#[derive(Clone)]
pub struct Dav {
    auth: String,
    /// What errors call the server: "Fastmail".
    pub name: String,
}

/// Which home to look for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Calendars,
    Contacts,
}

impl Kind {
    fn well_known(self) -> &'static str {
        match self {
            Kind::Calendars => "/.well-known/caldav",
            Kind::Contacts => "/.well-known/carddav",
        }
    }

    fn home_set(self) -> (&'static str, &'static str) {
        match self {
            Kind::Calendars => (CALDAV, "calendar-home-set"),
            Kind::Contacts => (CARDDAV, "addressbook-home-set"),
        }
    }
}

/// One address book (or calendar) in a home.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Collection {
    /// Its full URL, which is also its id.
    pub href: String,
    pub name: String,
    /// Changes whenever anything in it does, where the server says.
    pub ctag: Option<String>,
}

/// A card or event resource: where it is, its version, its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub href: String,
    pub etag: String,
    pub data: String,
}

const PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;

impl Dav {
    pub fn new(name: &str, user: &str, password: &str) -> Dav {
        // Google and Apple show app passwords in groups; the spaces aren't part of them.
        let password: String = password.split_whitespace().collect();
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{}:{password}", user.trim()));
        Dav { auth: format!("Basic {auth}"), name: name.to_owned() }
    }

    /// Sends a request, following redirects with the same method (ureq would turn them into
    /// GETs), only to the same host so the password never goes elsewhere.
    pub fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: &str) -> Result<Reply> {
        let agent = ureq::AgentBuilder::new().redirects(0).timeout(std::time::Duration::from_secs(30)).build();
        let mut url = url.to_owned();
        for _ in 0..5 {
            let mut req = agent.request(method, &url).set("Authorization", &self.auth);
            for (k, v) in headers {
                req = req.set(k, v);
            }
            let resp = match req.send_string(body) {
                Ok(r) => r,
                Err(ureq::Error::Status(code, r)) if (300..400).contains(&code) => r,
                Err(ureq::Error::Status(code, _)) => return Err(self.status_error(code, &url)),
                Err(e) => return Err(format!("{}: {e}", self.name)),
            };
            let status = resp.status();
            if (300..400).contains(&status) {
                let next = resp.header("Location").map(|l| join(&url, l)).ok_or("A redirect without an address.")?;
                if same_origin(&url, &next) {
                    url = next;
                    continue;
                }
                return Err(format!("{} sent the request to another server ({next}).", self.name));
            }
            let etag = resp.header("ETag").map(str::to_owned);
            let body = resp.into_string().map_err(|e| e.to_string())?;
            return Ok(Reply { status, etag, body });
        }
        Err(format!("{} redirected too many times.", self.name))
    }

    fn status_error(&self, code: u16, url: &str) -> String {
        match code {
            401 | 403 => format!("{} didn't accept that address and app password.", self.name),
            404 => format!("Nothing at {url}."),
            412 => "It changed on the server meanwhile; refresh and try again.".to_owned(),
            c => format!("{} answered {c}.", self.name),
        }
    }

    /// A PROPFIND or REPORT with an XML body.
    pub fn xml(&self, method: &str, url: &str, depth: &str, body: &str) -> Result<String> {
        let headers = [("Depth", depth), ("Content-Type", "application/xml; charset=utf-8")];
        Ok(self.send(method, url, &headers, body)?.body)
    }

    /// The home of the signed-in user's calendars or address books, found from `start`.
    pub fn home(&self, start: &str, kind: Kind) -> Result<String> {
        let start = start.trim();
        if start.is_empty() {
            return Err("Enter the server's address.".into());
        }
        let principal = |at: &str| -> Result<String> {
            let xml = self.xml("PROPFIND", at, "0", PRINCIPAL)?;
            first_href(&xml, "current-user-principal").map(|h| join(at, &h)).ok_or_else(|| "no principal".to_owned())
        };
        let principal = match principal(start) {
            Ok(p) => p,
            Err(e) if !start.contains(".well-known") => principal(&join(start, kind.well_known())).map_err(|_| e)?,
            Err(e) => return Err(e),
        };
        let (ns, name) = kind.home_set();
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:x="{ns}"><d:prop><x:{name}/></d:prop></d:propfind>"#
        );
        let xml = self.xml("PROPFIND", &principal, "0", &body)?;
        let home = first_href(&xml, name).ok_or_else(|| format!("{} has nothing of this kind for this account.", self.name))?;
        Ok(join(&principal, &home))
    }

    /// The address books in `home`.
    pub fn address_books(&self, home: &str) -> Result<Vec<Collection>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop><d:resourcetype/><d:displayname/><cs:getctag/></d:prop>
</d:propfind>"#;
        let xml = self.xml("PROPFIND", home, "1", body)?;
        collections(&xml, home, (CARDDAV, "addressbook"))
    }

    /// Every card in an address book, with its ETag.
    pub fn cards(&self, book: &str) -> Result<Vec<Resource>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:getetag/><c:address-data/></d:prop>
</c:addressbook-query>"#;
        let xml = self.xml("REPORT", book, "1", body)?;
        resources(&xml, book, (CARDDAV, "address-data"))
    }

    /// Writes a card: a new one when `etag` is None (refused if one's already there), else a
    /// change refused if the card changed since `etag`. Returns its new ETag when the server
    /// gives one.
    pub fn put_card(&self, href: &str, vcard: &str, etag: Option<&str>) -> Result<Option<String>> {
        let mut headers = vec![("Content-Type", "text/vcard; charset=utf-8")];
        match etag {
            Some(tag) => headers.push(("If-Match", tag)),
            None => headers.push(("If-None-Match", "*")),
        }
        Ok(self.send("PUT", href, &headers, vcard)?.etag)
    }

    /// Deletes a resource, unless it changed since `etag`.
    pub fn delete(&self, href: &str, etag: Option<&str>) -> Result<()> {
        let headers: Vec<(&str, &str)> = etag.map(|t| ("If-Match", t)).into_iter().collect();
        match self.send("DELETE", href, &headers, "") {
            Ok(_) => Ok(()),
            Err(e) if e.starts_with("Nothing at") => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// `href` made absolute against `base`.
pub fn join(base: &str, href: &str) -> String {
    url::Url::parse(base).and_then(|b| b.join(href.trim())).map_or_else(|_| href.trim().to_owned(), |u| u.to_string())
}

/// Whether two URLs share a scheme, host and port.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

/// The href inside the first element named `name`.
pub fn first_href(xml: &str, name: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let prop = doc.descendants().find(|n| n.tag_name().name() == name)?;
    prop.descendants().find(|n| n.has_tag_name((DAV, "href")))?.text().map(|t| t.trim().to_owned())
}

/// The collections of a PROPFIND's answer whose resource type includes `kind`, on the same
/// server as `base`.
pub fn collections(xml: &str, base: &str, kind: (&str, &str)) -> Result<Vec<Collection>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let find = |ns: &str, name: &str| resp.descendants().find(|n| n.has_tag_name((ns, name)));
        let is_kind = find(DAV, "resourcetype").is_some_and(|r| r.children().any(|c| c.has_tag_name(kind)));
        let Some(href) = find(DAV, "href").and_then(|h| h.text()) else { continue };
        let href = join(base, href);
        if is_kind && same_origin(base, &href) {
            out.push(Collection {
                href,
                name: find(DAV, "displayname").and_then(|n| n.text()).map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "Contacts".into()),
                ctag: find(CALSERVER, "getctag").and_then(|n| n.text()).map(|t| t.trim().to_owned()),
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// The resources of a REPORT's answer: href, ETag and the data in `data`.
pub fn resources(xml: &str, base: &str, data: (&str, &str)) -> Result<Vec<Resource>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let find = |ns: &str, name: &str| resp.descendants().find(|n| n.has_tag_name((ns, name)));
        let (Some(href), Some(body)) = (find(DAV, "href").and_then(|h| h.text()), find(data.0, data.1)) else { continue };
        let href = join(base, href);
        if !same_origin(base, &href) {
            continue;
        }
        out.push(Resource {
            href,
            etag: find(DAV, "getetag").and_then(|n| n.text()).map(|t| t.trim().to_owned()).unwrap_or_default(),
            data: body.children().filter_map(|c| c.text()).collect(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOKS: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:cs="http://calendarserver.org/ns/">
  <d:response><d:href>/dav/addressbooks/user/me/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
  <d:response><d:href>/dav/addressbooks/user/me/Default/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/><c:addressbook/></d:resourcetype>
      <d:displayname>Personal</d:displayname><cs:getctag>42</cs:getctag></d:prop></d:propstat></d:response>
  <d:response><d:href>https://evil.example/book/</d:href>
    <d:propstat><d:prop><d:resourcetype><c:addressbook/></d:resourcetype></d:prop></d:propstat></d:response>
</d:multistatus>"#;

    #[test]
    fn finds_address_books_on_the_same_server() {
        let got = collections(BOOKS, "https://carddav.example.com/dav/addressbooks/user/me/", (CARDDAV, "addressbook")).unwrap();
        assert_eq!(got.len(), 1, "the home itself and another server's book are left out");
        assert_eq!(got[0].href, "https://carddav.example.com/dav/addressbooks/user/me/Default/");
        assert_eq!((got[0].name.as_str(), got[0].ctag.as_deref()), ("Personal", Some("42")));
    }

    #[test]
    fn reads_cards_with_etags() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:response><d:href>/book/a.vcf</d:href><d:propstat><d:prop><d:getetag>"1"</d:getetag>
    <c:address-data>BEGIN:VCARD&#13;
FN:A&#13;
END:VCARD</c:address-data></d:prop></d:propstat></d:response></d:multistatus>"#;
        let got = resources(xml, "https://dav.example.com/book/", (CARDDAV, "address-data")).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].href.as_str(), got[0].etag.as_str()), ("https://dav.example.com/book/a.vcf", "\"1\""));
        assert!(got[0].data.contains("FN:A"));
    }

    #[test]
    fn joins_and_compares_origins() {
        assert_eq!(join("https://a.example/x/y/", "/z/"), "https://a.example/z/");
        assert!(same_origin("https://a.example/x", "https://a.example/y"));
        assert!(!same_origin("https://a.example/x", "http://a.example/x"));
        assert_eq!(first_href(PRINCIPAL_REPLY, "current-user-principal").as_deref(), Some("/principals/me/"));
    }

    const PRINCIPAL_REPLY: &str = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/</d:href><d:propstat><d:prop>
<d:current-user-principal><d:href>/principals/me/</d:href></d:current-user-principal></d:prop></d:propstat></d:response></d:multistatus>"#;
}
