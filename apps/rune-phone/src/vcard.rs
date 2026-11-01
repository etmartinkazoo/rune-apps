//! vCards as the phone hands them over PBAP: its contacts, and its call history (which carries
//! each call's time and kind in `X-IRMC-CALL-DATETIME`).

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Contact {
    pub name: String,
    /// (label, number): "mobile", "home", "work"…
    pub numbers: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CallKind {
    Incoming,
    Outgoing,
    Missed,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Call {
    pub name: String,
    pub number: String,
    pub kind: CallKind,
    /// Local time, as `YYYYMMDDTHHMMSS`.
    pub at: String,
}

/// Unfolds lines (a line starting with a space continues the one before) and yields each
/// card's lines.
fn cards(text: &str) -> Vec<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n').map(|l| l.trim_end_matches('\r')) {
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().expect("checked").push_str(rest),
            _ => lines.push(raw.to_owned()),
        }
    }
    let mut out = Vec::new();
    let mut cur: Option<Vec<String>> = None;
    for l in lines {
        if l.eq_ignore_ascii_case("BEGIN:VCARD") {
            cur = Some(Vec::new());
        } else if l.eq_ignore_ascii_case("END:VCARD") {
            out.extend(cur.take());
        } else if let Some(c) = cur.as_mut() {
            c.push(l);
        }
    }
    out
}

/// A line's name, its parameters (upper-cased) and value.
fn split(line: &str) -> (String, String, String) {
    let (head, value) = line.split_once(':').unwrap_or((line, ""));
    let mut parts = head.split(';');
    let name = parts.next().unwrap_or("").to_ascii_uppercase();
    let params = parts.collect::<Vec<_>>().join(";").to_ascii_uppercase();
    let value = decode(value, &params);
    (name, params, value)
}

/// A quoted-printable value (vCard 2.1) decoded; others as they are.
fn decode(value: &str, params: &str) -> String {
    if !params.contains("QUOTED-PRINTABLE") {
        return value.replace("\\,", ",").replace("\\;", ";");
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

fn name_of(card: &[String]) -> String {
    let mut fn_name = String::new();
    let mut n_name = String::new();
    for l in card {
        let (name, _, value) = split(l);
        match name.as_str() {
            "FN" => fn_name = value.trim().to_owned(),
            "N" => {
                let mut parts: Vec<&str> = value.split(';').take(2).filter(|s| !s.is_empty()).collect();
                parts.reverse();
                n_name = parts.join(" ");
            }
            _ => {}
        }
    }
    if fn_name.is_empty() { n_name } else { fn_name }
}

pub fn contacts(text: &str) -> Vec<Contact> {
    cards(text)
        .into_iter()
        .filter_map(|card| {
            let numbers: Vec<(String, String)> = card
                .iter()
                .filter_map(|l| {
                    let (name, params, value) = split(l);
                    (name == "TEL" && !value.trim().is_empty()).then(|| (label(&params), value.trim().to_owned()))
                })
                .collect();
            let name = name_of(&card);
            (!numbers.is_empty()).then_some(Contact { name, numbers })
        })
        .collect()
}

fn label(params: &str) -> String {
    for (key, label) in [("CELL", "mobile"), ("HOME", "home"), ("WORK", "work"), ("MAIN", "main")] {
        if params.contains(key) {
            return label.into();
        }
    }
    "phone".into()
}

/// The call history (PBAP's `cch`, or `ich`, `och`, `mch`), newest first as the phone sends it.
pub fn calls(text: &str) -> Vec<Call> {
    cards(text)
        .into_iter()
        .filter_map(|card| {
            let mut kind = None;
            let mut at = String::new();
            let mut number = String::new();
            for l in &card {
                let (name, params, value) = split(l);
                match name.as_str() {
                    "X-IRMC-CALL-DATETIME" => {
                        kind = Some(if params.contains("MISSED") {
                            CallKind::Missed
                        } else if params.contains("DIALED") {
                            CallKind::Outgoing
                        } else {
                            CallKind::Incoming
                        });
                        at = value.trim().to_owned();
                    }
                    "TEL" if number.is_empty() => number = value.trim().to_owned(),
                    _ => {}
                }
            }
            Some(Call { name: name_of(&card), number, kind: kind?, at })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_contacts_and_history() {
        let pb = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nN:Lovelace;Ada;;;\r\nTEL;TYPE=CELL:+44 7700 900123\r\n\
            TEL;TYPE=HOME:020 7946 0000\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=C3=89mile;\r\n\
            TEL;CELL:555 0100\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Number\r\nEND:VCARD\r\n";
        let c = contacts(pb);
        assert_eq!(c.len(), 2, "a card without a number isn't a contact here");
        assert_eq!(c[0].numbers, [("mobile".into(), "+44 7700 900123".into()), ("home".into(), "020 7946 0000".into())]);
        assert_eq!(c[1].name, "Émile");
        let cch = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nTEL:+447700900123\r\nX-IRMC-CALL-DATETIME;MISSED:20261031T091500\r\nEND:VCARD\r\n\
            BEGIN:VCARD\r\nVERSION:3.0\r\nFN:\r\nTEL:5550100\r\nX-IRMC-CALL-DATETIME;DIALED:20261030T180000\r\nEND:VCARD\r\n";
        let h = calls(cch);
        assert_eq!(h[0].kind, CallKind::Missed);
        assert_eq!(h[0].at, "20261031T091500");
        assert_eq!((h[1].kind, h[1].number.as_str()), (CallKind::Outgoing, "5550100"));
    }
}
