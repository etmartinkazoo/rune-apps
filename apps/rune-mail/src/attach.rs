//! Attachments and pasted images in compose.
//!
//! Files come from the Attach button (the system's file chooser), or by dropping them on the
//! compose sheet. An image pasted into the body goes in where the caret is, as a pill
//! ("Image 1") in the text; sending makes the message HTML with each image inline at its
//! pill (`cid:` parts in multipart/related), with the plain text alongside. Delete the pill
//! and that image isn't sent.

use std::path::Path;
use std::sync::Arc;

use rune::Tx;

use crate::app::App;

/// Over this, a message isn't sent (most servers refuse messages near 25 MB). It counts what
/// goes over the wire: see [`sent_size`].
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

/// What `bytes` of attachment take in the message: base64, in lines of 76 and a line break.
pub fn sent_size(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4 / 76 * 78 + 78
}

#[derive(Clone)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
    /// Set for an image pasted into the body: its Content-ID, which its pill names.
    pub cid: Option<String>,
    /// A thumbnail for images.
    pub thumb: Option<rune::Image>,
}

impl Attachment {
    pub fn size_label(&self) -> String {
        let n = self.bytes.len() as f64;
        if n < 1024.0 {
            format!("{n} B")
        } else if n < 1024.0 * 1024.0 {
            format!("{:.0} KB", n / 1024.0)
        } else {
            format!("{:.1} MB", n / 1024.0 / 1024.0)
        }
    }

    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

/// An attachment as sent: name, type, bytes and, for inline images, the Content-ID.
#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub name: String,
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
    pub cid: Option<String>,
}

pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ics" => "text/calendar",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The body as HTML, with each image pill replaced by its inline image; None when there are
/// no pasted images (the message stays plain text).
pub fn html_body(text: &str, pills: &[(std::ops::Range<usize>, String)]) -> Option<String> {
    let images: Vec<(std::ops::Range<usize>, &str)> = pills
        .iter()
        .filter_map(|(r, id)| id.strip_prefix("image:").map(|cid| (r.clone(), cid)))
        .collect();
    if images.is_empty() {
        return None;
    }
    let mut html = String::from("<div style=\"font-family: -apple-system, system-ui, sans-serif; font-size: 14px\">");
    let mut at = 0;
    let mut sorted = images;
    sorted.sort_by_key(|(r, _)| r.start);
    for (r, cid) in sorted {
        html.push_str(&escape(&text[at..r.start]).replace('\n', "<br>\n"));
        html.push_str(&format!("<img src=\"cid:{}\" style=\"max-width: 100%\">", escape(cid)));
        at = r.end;
    }
    html.push_str(&escape(&text[at..]).replace('\n', "<br>\n"));
    html.push_str("</div>");
    Some(html)
}

impl App {
    /// The Attach button: the system's file chooser, then the file.
    pub fn pick_attachment(&mut self, tx: &mut Tx) {
        let options = rune::FileDialogOptions { title: "Attach a File".into(), ..Default::default() };
        tx.open_file(options, |s: &mut App, result, tx| match result {
            Ok(Some(path)) => s.attach_file(&path, tx),
            Ok(None) => {}
            Err(e) => s.say(format!("Couldn't open the file chooser: {e}"), tx),
        });
    }

    pub fn attach_file(&mut self, path: &Path, tx: &mut Tx) {
        let name = path.file_name().map_or_else(|| "attachment".into(), |n| n.to_string_lossy().into_owned());
        let too_big = || format!("{name} is too big to send: mail takes it as a third larger, over 25 MB");
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => return self.say(format!("{name} is a folder"), tx),
            // A pipe or a device would never finish reading.
            Ok(m) if !m.is_file() => return self.say(format!("{name} isn't a file that can be attached"), tx),
            Ok(m) if usize::try_from(m.len()).map_or(true, |n| sent_size(n) > MAX_BYTES) => return self.say(too_big(), tx),
            Err(e) => return self.say(format!("Couldn't read {name}: {e}"), tx),
            _ => {}
        }
        // At most what can be sent, in case it grew since.
        let read = std::fs::File::open(path).and_then(|f| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(f, MAX_BYTES as u64 + 1), &mut bytes).map(|_| bytes)
        });
        match read {
            Ok(bytes) if sent_size(bytes.len()) > MAX_BYTES => self.say(too_big(), tx),
            Ok(bytes) => {
                let mime = mime_for(&name).to_owned();
                let a = self.attachment(name, mime, bytes, None);
                self.compose.attachments.push(a);
            }
            Err(e) => self.say(format!("Couldn't read {name}: {e}"), tx),
        }
    }

    /// An image pasted into the body: inline, at the caret, as a pill.
    pub fn paste_image(&mut self, png: Vec<u8>) {
        let n = self.compose.attachments.iter().filter(|a| a.cid.is_some()).count() + 1;
        let cid = format!("image{n}.{:x}@rune-mail", fastrand::u64(..));
        let a = self.attachment(format!("image-{n}.png"), "image/png".into(), png, Some(cid.clone()));
        self.compose.attachments.push(a);
        self.compose.body.insert_pill(format!("image:{cid}"), &format!("Image {n}"));
    }

    fn attachment(&self, name: String, mime: String, bytes: Vec<u8>, cid: Option<String>) -> Attachment {
        let bytes = Arc::new(bytes);
        let thumb = mime.starts_with("image/").then(|| self.load_image(bytes.as_ref().clone())).flatten();
        Attachment { name, mime, bytes, cid, thumb }
    }

    pub fn remove_attachment(&mut self, i: usize) {
        if i < self.compose.attachments.len() {
            self.compose.attachments.remove(i);
        }
    }

    /// What goes with the message: files, and the pasted images whose pills are still there.
    pub(crate) fn outgoing_attachments(&self) -> (Option<String>, Vec<Outgoing>) {
        let pills: Vec<(std::ops::Range<usize>, String)> =
            self.compose.body.pills().map(|(r, id)| (r, id.to_owned())).collect();
        let html = html_body(&self.compose.body.text, &pills);
        let live = |cid: &str| pills.iter().any(|(_, id)| id.strip_prefix("image:") == Some(cid));
        let list = self
            .compose
            .attachments
            .iter()
            .filter(|a| a.cid.as_deref().is_none_or(live))
            .map(|a| Outgoing { name: a.name.clone(), mime: a.mime.clone(), bytes: a.bytes.clone(), cid: a.cid.clone() })
            .collect();
        (html, list)
    }
}

#[cfg(test)]
mod tests {
    use super::sent_size;

    #[test]
    fn counts_attachments_as_they_go_over_the_wire() {
        assert!(sent_size(18 * 1024 * 1024) < super::MAX_BYTES);
        assert!(sent_size(20 * 1024 * 1024) > super::MAX_BYTES, "20 MB is some 27 MB once encoded");
    }

    use super::*;

    #[test]
    fn places_images_at_their_pills_and_escapes_text() {
        let text = "Look <here>:\nImage 1\nThanks";
        let at = text.find("Image 1").unwrap();
        let html = html_body(text, &[(at..at + 7, "image:abc@x".into())]).unwrap();
        assert!(html.contains("Look &lt;here&gt;:<br>"));
        assert!(html.contains("<img src=\"cid:abc@x\""));
        assert!(html.find("cid:abc@x").unwrap() < html.find("Thanks").unwrap());
        assert_eq!(html_body("no images", &[]), None);
    }

    #[test]
    fn guesses_types() {
        assert_eq!(mime_for("Report.PDF"), "application/pdf");
        assert_eq!(mime_for("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }
}
