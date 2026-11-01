//! vCards (2.1, 3.0 and 4.0) read into [`Card`]s and written back as 3.0.
//!
//! Reading unfolds lines, decodes quoted-printable (phones send 2.1 over PBAP), takes Apple's
//! `itemN.X-ABLabel` labels, and keeps every line it doesn't model in [`Card::other`], so a
//! card saved back loses nothing it came with (photos, anniversaries, `X-` fields).

use serde::{Deserialize, Serialize};

/// A value and what kind it is: "mobile", "home", "work", or a label of your own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}

impl Labeled {
    pub fn new(label: &str, value: &str) -> Labeled {
        Labeled { label: label.to_owned(), value: value.to_owned() }
    }
}

/// A postal address, in vCard's parts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub label: String,
    pub street: String,
    pub city: String,
    pub region: String,
    pub code: String,
    pub country: String,
}

impl Address {
    /// As lines to show: street, then "city region code", then country.
    pub fn lines(&self) -> Vec<String> {
        let middle = [self.city.as_str(), self.region.as_str(), self.code.as_str()]
            .into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" ");
        [self.street.clone(), middle, self.country.clone()].into_iter().flat_map(|l| {
            l.lines().map(str::to_owned).collect::<Vec<_>>()
        }).filter(|l| !l.trim().is_empty()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lines().is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    pub uid: String,
    pub given: String,
    pub family: String,
    /// FN as the card has it; empty to make it from the name parts or the company.
    pub formatted: String,
    pub org: String,
    pub title: String,
    pub emails: Vec<Labeled>,
    pub phones: Vec<Labeled>,
    pub addresses: Vec<Address>,
    pub urls: Vec<Labeled>,
    /// As written: `1990-04-01`, or `--04-01` without a year.
    pub birthday: String,
    pub note: String,
    /// Lines this doesn't model, kept as they came.
    pub other: Vec<String>,
}

impl Card {
    /// What to call them: FN, else given and family name, else the company, else an address.
    pub fn name(&self) -> String {
        if !self.formatted.trim().is_empty() {
            return self.formatted.trim().to_owned();
        }
        let parts = [self.given.trim(), self.family.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        if !parts.is_empty() {
            return parts;
        }
        if !self.org.trim().is_empty() {
            return self.org.trim().to_owned();
        }
        self.emails.first().or(self.phones.first()).map(|l| l.value.clone()).unwrap_or_default()
    }

    /// Sorts by family name, then given name, as Contacts lists them.
    pub fn sort_key(&self) -> String {
        let key = if self.family.trim().is_empty() { self.name() } else { format!("{} {}", self.family.trim(), self.given.trim()) };
        key.to_lowercase()
    }

    /// The card as vCard 3.0 text, CRLF line ends, long lines folded.
    pub fn to_vcard(&self) -> String {
        let mut lines = vec!["BEGIN:VCARD".to_owned(), "VERSION:3.0".to_owned()];
        lines.push(format!("UID:{}", escape(&self.uid)));
        lines.push(format!("FN:{}", escape(&self.name())));
        lines.push(format!("N:{};{};;;", escape(&self.family), escape(&self.given)));
        if !self.org.is_empty() {
            lines.push(format!("ORG:{}", escape(&self.org)));
        }
        if !self.title.is_empty() {
            lines.push(format!("TITLE:{}", escape(&self.title)));
        }
        // A label vCard has no TYPE for goes in Apple's way: `itemN.EMAIL` and `itemN.X-ABLabel`,
        // numbered past the groups of the lines kept as they came.
        let mut group = self.other.iter().filter_map(|l| {
            let g = l.split(['.', ';', ':']).next()?;
            g.get(..4).filter(|p| p.eq_ignore_ascii_case("item"))?;
            g[4..].parse::<u32>().ok()
        }).max().unwrap_or(0);
        let mut put = |lines: &mut Vec<String>, name: &str, extra: &str, label: &str, value: String| {
            let t = type_param(label);
            let plain = ["", "email", "phone", "address", "homepage"].contains(&label.trim().to_lowercase().as_str());
            if t.is_empty() && !plain {
                group += 1;
                lines.push(format!("item{group}.{name}{extra}:{value}"));
                lines.push(format!("item{group}.X-ABLabel:{}", escape(label.trim())));
            } else {
                lines.push(format!("{name}{extra}{t}:{value}"));
            }
        };
        for e in self.emails.iter().filter(|e| !e.value.trim().is_empty()) {
            put(&mut lines, "EMAIL", ";TYPE=INTERNET", &e.label, escape(e.value.trim()));
        }
        for t in self.phones.iter().filter(|t| !t.value.trim().is_empty()) {
            put(&mut lines, "TEL", "", &t.label, escape(t.value.trim()));
        }
        for a in self.addresses.iter().filter(|a| !a.is_empty()) {
            let value = format!(";;{};{};{};{};{}", escape(&a.street), escape(&a.city), escape(&a.region), escape(&a.code), escape(&a.country));
            put(&mut lines, "ADR", "", &a.label, value);
        }
        for u in self.urls.iter().filter(|u| !u.value.trim().is_empty()) {
            put(&mut lines, "URL", "", &u.label, escape(u.value.trim()));
        }
        if !self.birthday.is_empty() {
            lines.push(format!("BDAY:{}", self.birthday));
        }
        if !self.note.is_empty() {
            lines.push(format!("NOTE:{}", escape(&self.note)));
        }
        lines.extend(self.other.iter().cloned());
        lines.push("END:VCARD".to_owned());
        lines.iter().map(|l| fold(l)).collect::<Vec<_>>().join("\r\n") + "\r\n"
    }
}

/// Every card in the text.
pub fn parse(text: &str) -> Vec<Card> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<String>> = None;
    for l in unfold(text) {
        if l.eq_ignore_ascii_case("BEGIN:VCARD") {
            cur = Some(Vec::new());
        } else if l.eq_ignore_ascii_case("END:VCARD") {
            if let Some(lines) = cur.take() {
                out.push(card(&lines));
            }
        } else if let Some(c) = cur.as_mut() {
            c.push(l);
        }
    }
    out
}

/// Joins continuation lines (a line starting with a space or tab continues the one before),
/// and quoted-printable soft breaks (`=` at the end of a line).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut soft = false;
    for raw in text.split('\n').map(|l| l.trim_end_matches('\r')) {
        if soft && let Some(last) = lines.last_mut() {
            last.pop();
            last.push_str(raw);
        } else {
            match raw.strip_prefix([' ', '\t']) {
                Some(rest) if !lines.is_empty() => lines.last_mut().expect("checked").push_str(rest),
                _ => lines.push(raw.to_owned()),
            }
        }
        let last = lines.last().map(String::as_str).unwrap_or("");
        soft = last.ends_with('=') && last.split(':').next().unwrap_or("").to_ascii_uppercase().contains("QUOTED-PRINTABLE");
    }
    lines
}

/// A line's group (`item1`), name, parameters (upper-cased) and raw value.
struct Line<'a> {
    group: Option<&'a str>,
    name: String,
    params: String,
    value: &'a str,
}

fn line(l: &str) -> Option<Line<'_>> {
    let (head, value) = split_value(l)?;
    let mut parts = head.split(';');
    let full = parts.next().unwrap_or("");
    let (group, name) = match full.split_once('.') {
        Some((g, n)) => (Some(g), n),
        None => (None, full),
    };
    Some(Line { group, name: name.to_ascii_uppercase(), params: parts.collect::<Vec<_>>().join(";").to_ascii_uppercase(), value })
}

/// Splits at the first colon outside a quoted parameter value.
fn split_value(l: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, ch) in l.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&l[..i], &l[i + 1..])),
            _ => {}
        }
    }
    None
}

fn card(lines: &[String]) -> Card {
    let parsed: Vec<(&String, Line)> = lines.iter().filter_map(|l| Some((l, line(l)?))).collect();
    // Apple's labels: `item1.X-ABLabel:Partner` names `item1.EMAIL`.
    let custom = |group: Option<&str>| -> Option<String> {
        let g = group?;
        parsed.iter().find(|(_, p)| p.group.is_some_and(|pg| pg.eq_ignore_ascii_case(g)) && p.name == "X-ABLABEL").map(|(_, p)| {
            let v = text(p.value, &p.params);
            v.trim_start_matches("_$!<").trim_end_matches(">!$_").to_lowercase()
        })
    };
    // Groups of the lines modelled here, whose labels are taken with them.
    let mut modelled: Vec<String> = Vec::new();
    let mut c = Card::default();
    for (raw, p) in &parsed {
        let v = || text(p.value, &p.params);
        let mut took = true;
        match p.name.as_str() {
            "VERSION" | "PRODID" | "X-ABLABEL" => took = false,
            "UID" => c.uid = v(),
            "FN" => c.formatted = v(),
            "N" => {
                let parts = components(p.value, &p.params);
                c.family = parts.first().cloned().unwrap_or_default();
                c.given = parts.get(1).cloned().unwrap_or_default();
            }
            "ORG" => c.org = components(p.value, &p.params).into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
            "TITLE" => c.title = v(),
            "EMAIL" => {
                let value = v().trim().trim_start_matches("mailto:").to_owned();
                if !value.is_empty() {
                    c.emails.push(Labeled { label: custom(p.group).unwrap_or_else(|| label(&p.params, "email")), value });
                }
            }
            "TEL" => {
                let value = v().trim().trim_start_matches("tel:").to_owned();
                if !value.is_empty() {
                    c.phones.push(Labeled { label: custom(p.group).unwrap_or_else(|| label(&p.params, "phone")), value });
                }
            }
            "ADR" => {
                let parts = components(p.value, &p.params);
                let get = |i: usize| parts.get(i).cloned().unwrap_or_default();
                // PO box;extended;street;city;region;code;country
                let street = [get(0), get(1), get(2)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
                c.addresses.push(Address {
                    label: custom(p.group).unwrap_or_else(|| label(&p.params, "address")),
                    street,
                    city: get(3),
                    region: get(4),
                    code: get(5),
                    country: get(6),
                });
            }
            "URL" => c.urls.push(Labeled { label: custom(p.group).unwrap_or_else(|| label(&p.params, "homepage")), value: v() }),
            "BDAY" => c.birthday = v(),
            "NOTE" => c.note = v(),
            _ => {
                c.other.push((*raw).clone());
                took = false;
            }
        }
        if took && let Some(g) = p.group {
            modelled.push(g.to_ascii_lowercase());
        }
    }
    // Labels of lines kept as they came stay with them.
    for (raw, p) in &parsed {
        if p.name == "X-ABLABEL" && p.group.is_some_and(|g| !modelled.contains(&g.to_ascii_lowercase())) {
            c.other.push((*raw).clone());
        }
    }
    c
}

/// What a TYPE parameter says, in Contacts' words.
fn label(params: &str, fallback: &str) -> String {
    let types: Vec<&str> = params
        .split(';')
        .flat_map(|p| p.strip_prefix("TYPE=").unwrap_or(p).split(','))
        .map(|t| t.trim_matches('"'))
        .collect();
    for (key, label) in [("CELL", "mobile"), ("IPHONE", "iPhone"), ("HOME", "home"), ("WORK", "work"), ("MAIN", "main"), ("FAX", "fax"), ("PAGER", "pager"), ("OTHER", "other")] {
        if types.iter().any(|t| t.eq_ignore_ascii_case(key)) {
            return label.to_owned();
        }
    }
    fallback.to_owned()
}

/// The TYPE parameter for a label: `;TYPE=CELL` for "mobile", `;TYPE=HOME`; nothing for others.
fn type_param(label: &str) -> String {
    let t = match label.to_lowercase().as_str() {
        "mobile" | "iphone" => "CELL",
        "home" => "HOME",
        "work" => "WORK",
        "main" => "MAIN",
        "fax" => "FAX",
        "pager" => "PAGER",
        "other" => "OTHER",
        _ => return String::new(),
    };
    format!(";TYPE={t}")
}

/// A text value: quoted-printable decoded, escapes undone.
fn text(value: &str, params: &str) -> String {
    unescape(&decode(value, params))
}

/// A structured value's parts (split at unescaped `;`), each unescaped.
fn components(value: &str, params: &str) -> Vec<String> {
    let value = decode(value, params);
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(n) = chars.next() {
                    parts.last_mut().expect("never empty").push_str(&format!("\\{n}"));
                }
            }
            ';' => parts.push(String::new()),
            c => parts.last_mut().expect("never empty").push(c),
        }
    }
    parts.iter().map(|p| unescape(p)).collect()
}

fn decode(value: &str, params: &str) -> String {
    if !params.contains("QUOTED-PRINTABLE") {
        return value.to_owned();
    }
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'='
            && let Some(b) = value.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(c) => out.push(c),
            None => out.push('\\'),
        }
    }
    out
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(',', "\\,").replace(';', "\\;").replace("\r\n", "\\n").replace('\n', "\\n")
}

/// Folds a line at 75 bytes, never inside a character.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += n;
    }
    out
}

/// A new UID for a card.
pub fn new_uid() -> String {
    let mut bytes = [0u8; 16];
    // Time and process mixed: unique enough for a UID made on this machine.
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let mut x = t ^ ((std::process::id() as u128) << 64);
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    x ^= (COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128) << 96;
    for b in &mut bytes {
        // xorshift over the seed
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]).to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_names_numbers_addresses_and_labels() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:abc\r\nFN:Grace Hopper\r\nN:Hopper;Grace;;;\r\nORG:US Navy;Research\r\n\
            item1.EMAIL;TYPE=INTERNET:grace@example.com\r\nitem1.X-ABLabel:_$!<Other>!$_\r\nEMAIL;TYPE=work:g.hopper@navy.exam\r\n ple\r\n\
            TEL;TYPE=CELL,VOICE:+1 555 0100\r\nADR;TYPE=HOME:;;1 Main St;Arlington;VA;22201;USA\r\n\
            NOTE:Line one\\nLine two\\, with comma\r\nPHOTO;VALUE=uri:https://example.com/p.jpg\r\nEND:VCARD\r\n";
        let c = &parse(text)[0];
        assert_eq!((c.uid.as_str(), c.name().as_str(), c.given.as_str(), c.family.as_str()), ("abc", "Grace Hopper", "Grace", "Hopper"));
        assert_eq!(c.org, "US Navy, Research");
        assert_eq!(c.emails, [Labeled::new("other", "grace@example.com"), Labeled::new("work", "g.hopper@navy.example")]);
        assert_eq!(c.phones, [Labeled::new("mobile", "+1 555 0100")]);
        assert_eq!(c.addresses[0].lines(), ["1 Main St", "Arlington VA 22201", "USA"]);
        assert_eq!(c.note, "Line one\nLine two, with comma");
        assert_eq!(c.other, ["PHOTO;VALUE=uri:https://example.com/p.jpg"], "what isn't modelled is kept");
    }

    #[test]
    fn reads_quoted_printable_from_phones() {
        let text = "BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=C3=89mile;\r\nTEL;CELL:555 0100\r\nEND:VCARD\r\n";
        let c = &parse(text)[0];
        assert_eq!(c.name(), "Émile");
        assert_eq!(c.phones[0].label, "mobile");
    }

    #[test]
    fn writes_what_it_reads() {
        let mut c = Card { uid: "u1".into(), given: "Ada".into(), family: "Lovelace".into(), ..Card::default() };
        c.emails.push(Labeled::new("home", "ada@example.com"));
        c.phones.push(Labeled::new("mobile", "+44 7700 900123"));
        c.note = "Analyst; poet, mathematician\nand more".into();
        c.addresses.push(Address { label: "work".into(), street: "12 St James's Sq".into(), city: "London".into(), ..Address::default() });
        c.other.push("X-SOCIAL:@ada".into());
        c.phones.push(Labeled::new("partner", "555 0199"));
        let text = c.to_vcard();
        assert!(text.contains("TEL;TYPE=CELL:+44 7700 900123\r\n"));
        assert!(text.contains("item1.TEL:555 0199\r\nitem1.X-ABLabel:partner\r\n"), "{text}");
        let back = parse(&text).remove(0);
        assert_eq!(back.formatted, "Ada Lovelace");
        assert_eq!(Card { formatted: String::new(), ..back }, c);
    }

    #[test]
    fn folds_long_lines_on_character_boundaries() {
        let c = Card { uid: "u".into(), note: "é".repeat(80), ..Card::default() };
        let text = c.to_vcard();
        assert!(text.split("\r\n").all(|l| l.len() <= 75));
        assert_eq!(parse(&text)[0].note, "é".repeat(80));
        assert_ne!(new_uid(), new_uid());
    }
}
