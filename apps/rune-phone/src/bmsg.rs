//! bMessage, the text format the Message Access Profile moves messages in (Bluetooth MAP 1.4,
//! section 3.1.3): the phone hands one over for each message read, and takes one for each sent.
//!
//! ```text
//! BEGIN:BMSG / VERSION / STATUS / TYPE / FOLDER
//!   originator vCards
//!   BEGIN:BENV  recipient vCards
//!     BEGIN:BBODY  CHARSET / LENGTH
//!       BEGIN:MSG … END:MSG
//! ```
//!
//! LENGTH counts the bytes from `BEGIN:MSG` through the line break after `END:MSG`.

/// One party of a message, from its vCard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Party {
    pub name: String,
    pub number: String,
}

/// A message as read from the phone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Who sent it (the first originator vCard).
    pub from: Party,
    /// Who it went to (the envelope's vCards).
    pub to: Vec<Party>,
    pub text: String,
    /// It carried parts other than text (an MMS photo, say).
    pub attachment: bool,
}

/// The bMessage that sends `text` as an SMS to `number`.
pub fn compose_sms(number: &str, text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let msg = format!("BEGIN:MSG\r\n{text}\r\nEND:MSG\r\n");
    format!(
        "BEGIN:BMSG\r\nVERSION:1.0\r\nSTATUS:READ\r\nTYPE:SMS_GSM\r\nFOLDER:TELECOM/MSG/OUTBOX\r\n\
         BEGIN:VCARD\r\nVERSION:2.1\r\nN:\r\nTEL:\r\nEND:VCARD\r\n\
         BEGIN:BENV\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nN:\r\nTEL:{number}\r\nEND:VCARD\r\n\
         BEGIN:BBODY\r\nCHARSET:UTF-8\r\nLENGTH:{len}\r\n{msg}END:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n",
        number = number.trim(),
        len = msg.len(),
    )
}

/// Reads a bMessage: its parties and its text. An MMS body is MIME; its text parts are kept.
pub fn parse(b: &str) -> Parsed {
    let lines: Vec<&str> = b.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut out = Parsed::default();
    let mut depth_env = 0;
    let mut card: Option<Party> = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let upper = line.to_ascii_uppercase();
        match upper.as_str() {
            "BEGIN:BENV" => depth_env += 1,
            "END:BENV" => depth_env -= 1,
            "BEGIN:VCARD" => card = Some(Party::default()),
            "END:VCARD" => {
                if let Some(p) = card.take() {
                    if depth_env == 0 {
                        if out.from == Party::default() {
                            out.from = p;
                        }
                    } else {
                        out.to.push(p);
                    }
                }
            }
            "BEGIN:MSG" => {
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() && !lines[i].eq_ignore_ascii_case("END:MSG") {
                    body.push(lines[i]);
                    i += 1;
                }
                let body = body.join("\n");
                let (text, attachment) = if body.to_ascii_lowercase().contains("content-type:") { mime_text(&body) } else { (body, false) };
                if !out.text.is_empty() && !text.is_empty() {
                    out.text.push('\n');
                }
                out.text.push_str(&text);
                out.attachment |= attachment;
            }
            _ => {
                if let Some(p) = card.as_mut() {
                    let (key, value) = line.split_once(':').unwrap_or((line, ""));
                    let key = key.split(';').next().unwrap_or("").to_ascii_uppercase();
                    match key.as_str() {
                        "FN" if !value.is_empty() => p.name = value.trim().to_owned(),
                        "N" if p.name.is_empty() => p.name = value.split(';').filter(|s| !s.is_empty()).rev().collect::<Vec<_>>().join(" "),
                        "TEL" if p.number.is_empty() => p.number = crate::number::address(value),
                        _ => {}
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// The text of a MIME body (an MMS, or an RCS chat message the phone keeps as one), and whether
/// it held anything else. Headers may be folded, parts nested, and text sent as base64 or
/// quoted-printable; a part with no Content-Type is text (RFC 2045). The SMIL layout an MMS
/// carries isn't an attachment.
fn mime_text(body: &str) -> (String, bool) {
    let (head, content) = split_head(body);
    let (kind, encoding) = (header(&head, "content-type"), header(&head, "content-transfer-encoding"));
    let lower = kind.to_ascii_lowercase();
    if lower.starts_with("multipart/") {
        let Some(boundary) = param(&kind, "boundary") else { return (content.trim().to_owned(), false) };
        let (mut texts, mut other) = (Vec::new(), false);
        for part in content.split(&format!("--{boundary}")).skip(1) {
            // What follows the closing `--boundary--`.
            if part.starts_with("--") {
                continue;
            }
            let part = part.strip_prefix('\n').unwrap_or(part);
            let (t, o) = mime_text(part);
            if !t.is_empty() {
                texts.push(t);
            }
            other |= o;
        }
        return (texts.join("\n"), other);
    }
    if lower.is_empty() || lower.starts_with("text/plain") {
        (decode(&content, &encoding).trim().to_owned(), false)
    } else if lower.starts_with("application/smil") {
        (String::new(), false)
    } else {
        (String::new(), true)
    }
}

/// A MIME entity's headers, unfolded, and what follows the blank line after them. A body with no
/// headers at all is all content.
fn split_head(entity: &str) -> (Vec<(String, String)>, String) {
    let looks_like_header = |l: &str| l.split_once(':').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if !entity.lines().next().is_some_and(looks_like_header) {
        return (Vec::new(), entity.to_owned());
    }
    let (head, content) = entity.split_once("\n\n").unwrap_or((entity, ""));
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = headers.last_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    (headers, content.to_owned())
}

fn header(headers: &[(String, String)], name: &str) -> String {
    headers.iter().find(|(k, _)| k == name).map_or(String::new(), |(_, v)| v.clone())
}

/// A parameter of a header value: `boundary` of `multipart/related; boundary="b1"`.
fn param(value: &str, name: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().trim_matches('"').to_owned())
    }).filter(|v| !v.is_empty())
}

/// Content as sent: base64 and quoted-printable undone, anything else as it is.
fn decode(content: &str, encoding: &str) -> String {
    use base64::Engine;
    match encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => {
            let packed: String = content.chars().filter(|c| !c.is_whitespace()).collect();
            match base64::engine::general_purpose::STANDARD_NO_PAD.decode(packed.trim_end_matches('=')) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(_) => content.to_owned(),
            }
        }
        "quoted-printable" => {
            let mut bytes = Vec::new();
            let mut lines = content.split('\n').peekable();
            while let Some(line) = lines.next() {
                let (line, soft) = match line.strip_suffix('=') {
                    Some(l) => (l, true),
                    None => (line, false),
                };
                let b = line.as_bytes();
                let mut i = 0;
                while i < b.len() {
                    if b[i] == b'='
                        && let Some(v) = line.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
                    {
                        bytes.push(v);
                        i += 3;
                        continue;
                    }
                    bytes.push(b[i]);
                    i += 1;
                }
                if !soft && lines.peek().is_some() {
                    bytes.push(b'\n');
                }
            }
            String::from_utf8_lossy(&bytes).into_owned()
        }
        _ => content.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_what_the_profile_asks() {
        let b = compose_sms("+15551234567", "Running late\nsee you at 7");
        assert!(b.starts_with("BEGIN:BMSG\r\nVERSION:1.0\r\n"));
        assert!(b.contains("TYPE:SMS_GSM\r\n") && b.contains("TEL:+15551234567\r\n"));
        let msg = "BEGIN:MSG\r\nRunning late\r\nsee you at 7\r\nEND:MSG\r\n";
        assert!(b.contains(&format!("LENGTH:{}\r\n{msg}", msg.len())), "LENGTH counts BEGIN:MSG to END:MSG's line break");
        assert!(b.ends_with("END:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n"));
        let back = parse(&b);
        assert_eq!(back.text, "Running late\nsee you at 7");
        assert_eq!(back.to, [Party { name: String::new(), number: "+15551234567".into() }]);
    }

    #[test]
    fn reads_a_received_sms_and_an_mms() {
        let sms = "BEGIN:BMSG\r\nVERSION:1.0\r\nSTATUS:UNREAD\r\nTYPE:SMS_GSM\r\nFOLDER:TELECOM/MSG/INBOX\r\n\
            BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nN:Lovelace;Ada\r\nTEL:+447700900123\r\nEND:VCARD\r\n\
            BEGIN:BENV\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:\r\nTEL:+15550001111\r\nEND:VCARD\r\n\
            BEGIN:BBODY\r\nCHARSET:UTF-8\r\nLENGTH:30\r\nBEGIN:MSG\r\nOn my way 🚲\r\nEND:MSG\r\nEND:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n";
        let p = parse(sms);
        assert_eq!(p.from, Party { name: "Ada Lovelace".into(), number: "+447700900123".into() });
        assert_eq!(p.text, "On my way 🚲");
        assert!(!p.attachment);
        let mms = "BEGIN:BMSG\r\nTYPE:MMS\r\nBEGIN:VCARD\r\nN:Hopper;Grace\r\nTEL:555\r\nEND:VCARD\r\nBEGIN:BENV\r\nBEGIN:BBODY\r\nBEGIN:MSG\r\n\
            Content-Type: multipart/related; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nLook at this\r\n\
            --b1\r\nContent-Type: image/jpeg\r\n\r\nAAAA\r\n--b1--\r\nEND:MSG\r\nEND:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n";
        let p = parse(mms);
        assert_eq!(p.from.name, "Grace Hopper", "N when there's no FN");
        assert_eq!(p.text, "Look at this");
        assert!(p.attachment);
    }

    fn body(msg: &str) -> String {
        format!("BEGIN:BMSG\r\nTYPE:MMS\r\nBEGIN:VCARD\r\nTEL:sip:+15550100123@ims.example.org\r\nEND:VCARD\r\nBEGIN:BENV\r\nBEGIN:BBODY\r\nBEGIN:MSG\r\n{msg}\r\nEND:MSG\r\nEND:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n")
    }

    #[test]
    fn reads_rcs_chat_messages_kept_as_mms() {
        // Folded Content-Type, a SMIL part, and the text in base64: no attachment, and the text.
        let p = parse(&body(
            "Date: Tue, 10 Nov 2026 09:00:00 -0800\r\nContent-Type: multipart/related;\r\n boundary=\"MessageBoundary.1\"\r\nMime-Version: 1.0\r\n\r\n\
             --MessageBoundary.1\r\nContent-Type: application/smil\r\n\r\n<smil/>\r\n\
             --MessageBoundary.1\r\nContent-Type: text/plain; charset=\"UTF-8\"\r\nContent-Transfer-Encoding: BASE64\r\n\r\nU2VlIHlvdSBhdCA3IPCfmoI=\r\n\
             --MessageBoundary.1--",
        ));
        assert_eq!(p.text, "See you at 7 🚂");
        assert!(!p.attachment, "SMIL isn't an attachment");
        assert_eq!(p.from.number, "+15550100123", "a SIP address's domain isn't part of the number");
    }

    #[test]
    fn reads_quoted_printable_nested_and_one_part_bodies() {
        let p = parse(&body(
            "Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n\
             --inner\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCaf=C3=A9 at no=\r\non?\r\n--inner--\r\n\
             --outer\r\nContent-Type: image/jpeg\r\nContent-Transfer-Encoding: base64\r\n\r\n/9j/\r\n--outer--",
        ));
        assert_eq!(p.text, "Café at noon?");
        assert!(p.attachment);
        let p = parse(&body("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\naGk="));
        assert_eq!(p.text, "hi");
    }
}
