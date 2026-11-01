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
                        "TEL" if p.number.is_empty() => p.number = value.trim().to_owned(),
                        _ => {}
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// The text of a MIME body (an MMS), and whether it held anything else.
fn mime_text(body: &str) -> (String, bool) {
    let boundary = body.lines().find_map(|l| {
        let lower = l.to_ascii_lowercase();
        let at = lower.find("boundary=")?;
        Some(l[at + 9..].trim().trim_matches('"').split(';').next().unwrap_or("").to_owned())
    });
    let Some(boundary) = boundary.filter(|b| !b.is_empty()) else {
        // One part: its text after the headers.
        let text = body.split_once("\n\n").map_or(body, |(_, t)| t);
        return (text.trim().to_owned(), false);
    };
    let mut texts = Vec::new();
    let mut other = false;
    for part in body.split(&format!("--{boundary}")).skip(1) {
        let Some((head, content)) = part.split_once("\n\n") else { continue };
        let head = head.to_ascii_lowercase();
        if head.contains("content-type: text/plain") || head.contains("content-type:text/plain") {
            texts.push(content.trim().to_owned());
        } else if head.contains("content-type:") {
            other = true;
        }
    }
    (texts.join("\n"), other)
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
}
