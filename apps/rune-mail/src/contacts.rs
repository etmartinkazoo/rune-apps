//! Contacts over CardDAV, for address autocomplete. Fastmail's app passwords open CardDAV as
//! they do CalDAV; every address book on the account is read in one REPORT each.

use base64::Engine;

const HOST: &str = "https://carddav.fastmail.com";

/// One person and their addresses, from a vCard.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Contact {
    pub name: String,
    pub emails: Vec<String>,
}

#[derive(Clone)]
pub struct CardDav {
    user: String,
    auth: String,
}

impl CardDav {
    pub fn new(user: &str, password: &str) -> CardDav {
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        CardDav { user: user.to_owned(), auth: format!("Basic {auth}") }
    }

    fn request(&self, method: &str, url: &str, body: &str) -> Result<String, String> {
        ureq::request(method, url)
            .set("Authorization", &self.auth)
            .set("Depth", "1")
            .set("Content-Type", "application/xml; charset=utf-8")
            .timeout(std::time::Duration::from_secs(30))
            .send_string(body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => "Fastmail rejected the app password for contacts.".to_owned(),
                e => format!("Contacts: {e}"),
            })?
            .into_string()
            .map_err(|e| e.to_string())
    }

    /// Every contact with an address, from every address book.
    pub fn contacts(&self) -> Result<Vec<Contact>, String> {
        let home = format!("{HOST}/dav/addressbooks/user/{}/", self.user);
        let list = self.request(
            "PROPFIND",
            &home,
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#,
        )?;
        let doc = roxmltree::Document::parse(&list).map_err(|e| e.to_string())?;
        let books: Vec<String> = doc
            .descendants()
            .filter(|n| n.has_tag_name(("DAV:", "response")))
            .filter(|r| {
                r.descendants().any(|n| n.has_tag_name(("urn:ietf:params:xml:ns:carddav", "addressbook")))
            })
            .filter_map(|r| r.descendants().find(|n| n.has_tag_name(("DAV:", "href")))?.text().map(|t| t.trim().to_owned()))
            .collect();
        let query = r#"<?xml version="1.0" encoding="utf-8"?>
<c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop><c:address-data/></d:prop>
</c:addressbook-query>"#;
        let mut out = Vec::new();
        // Only on Fastmail's own host: anywhere else would be sent the app password.
        for url in books.iter().filter_map(|href| crate::caldav::on_host(HOST, href)) {
            let xml = self.request("REPORT", &url, query)?;
            let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
            for data in doc.descendants().filter(|n| n.has_tag_name(("urn:ietf:params:xml:ns:carddav", "address-data"))) {
                let card: String = data.children().filter_map(|c| c.text()).collect();
                out.extend(parse_vcard(&card));
            }
        }
        Ok(out)
    }
}

/// The contacts in vCard text (3.0 or 4.0): FN (else N) and every EMAIL.
pub fn parse_vcard(text: &str) -> Vec<Contact> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(raw.to_owned()),
        }
    }
    let mut out = Vec::new();
    let (mut name, mut structured, mut emails) = (String::new(), String::new(), Vec::new());
    for line in &lines {
        let Some((head, value)) = line.split_once(':') else { continue };
        // "item1.EMAIL;TYPE=work" → "EMAIL"
        let prop = head.split(';').next().unwrap_or(head);
        let prop = prop.rsplit('.').next().unwrap_or(prop).to_ascii_uppercase();
        match (prop.as_str(), value) {
            ("BEGIN", v) if v.eq_ignore_ascii_case("VCARD") => {
                name.clear();
                structured.clear();
                emails.clear();
            }
            ("FN", v) => name = unescape(v),
            ("N", v) => {
                // Family;Given;Middle;Prefix;Suffix
                let parts: Vec<String> = v.split(';').map(unescape).collect();
                structured = [parts.get(1), parts.first()].into_iter().flatten().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" ");
            }
            ("EMAIL", v) => {
                let v = v.trim().trim_start_matches("mailto:").to_owned();
                if v.contains('@') && !emails.contains(&v) {
                    emails.push(v);
                }
            }
            ("END", v) if v.eq_ignore_ascii_case("VCARD") && !emails.is_empty() => {
                let name = if name.trim().is_empty() { structured.clone() } else { name.trim().to_owned() };
                out.push(Contact { name, emails: std::mem::take(&mut emails) });
            }
            _ => {}
        }
    }
    out
}

fn unescape(v: &str) -> String {
    v.replace("\\,", ",").replace("\\;", ";").replace("\\n", " ").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_names_and_every_address() {
        let cards = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Grace Hopper\r\nN:Hopper;Grace;;;\r\n\
                     item1.EMAIL;TYPE=INTERNET:grace@example.com\r\nEMAIL;TYPE=work:g.hopper@navy.example\r\n\
                     END:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nN:Lovelace;Ada;;;\r\nEMAIL:ada@exam\r\n ple.com\r\n\
                     END:VCARD\r\nBEGIN:VCARD\r\nFN:No Address\r\nEND:VCARD\r\n";
        let c = parse_vcard(cards);
        assert_eq!(c.len(), 2, "cards without an address are skipped");
        assert_eq!(c[0], Contact { name: "Grace Hopper".into(), emails: vec!["grace@example.com".into(), "g.hopper@navy.example".into()] });
        assert_eq!(c[1].name, "Ada Lovelace", "N stands in for a missing FN");
        assert_eq!(c[1].emails, ["ada@example.com"], "folded lines are joined");
    }
}
