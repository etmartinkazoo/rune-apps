//! Text the reader draws itself: plain-text bodies with their links, and the colours a
//! message shows in. HTML is laid out and painted by `page.rs`.

use rune::color::hex;
use rune::{Color, Link, Span, TextStyle, Tx, El, rich};

use super::style::Palette;
use crate::app::App;
use crate::html::Doc;

/// Colours for one message: dark on a light page, or the app's theme.
#[derive(Clone, Copy)]
pub struct Ink {
    pub text: Color,
    pub link: Color,
    /// The highlight behind selected text.
    pub selection: Color,
}

impl Ink {
    /// The ink, and whether the message shows on a light page. `white`: the reader asked for
    /// this message on a white page.
    pub fn for_doc(doc: &Doc, white: bool, p: Palette) -> (Ink, bool) {
        let paper = !p.dark || doc.colored || white;
        let ink = if paper { Ink::paper() } else { Ink { text: p.text, link: p.accent, selection: p.text_selection } };
        (ink, paper)
    }

    /// Dark ink for a white page.
    pub fn paper() -> Ink {
        Ink { text: c("#1d1d1f"), link: c("#0b57d0"), selection: c("#0064e14d") }
    }
}

fn c(s: &str) -> Color {
    hex(s).expect("valid hex")
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Plain text with its URLs made into links.
pub fn plain(s: &str, style: TextStyle, color: Color, link: Color, selection: Color) -> El {
    let mut spans = Vec::new();
    let mut links = Vec::new();
    let mut i = 0;
    while let Some(off) = s[i..].find("http") {
        let start = i + off;
        let rest = &s[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            i = start + 4;
            continue;
        }
        let len = rest.find(|ch: char| ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | ')' | ']')).unwrap_or(rest.len());
        let end = start + rest[..len].trim_end_matches(['.', ',', ';', ':', '!', '?']).len();
        spans.push(Span::new(start, end).color(link));
        links.push(Link::new(start..end, s[start..end].to_owned()));
        i = end.max(start + 1);
    }
    let mut t = rich(s.to_owned(), spans).font(style).color(color).wrap().selectable(true).selection_color(selection).w_full();
    if !links.is_empty() {
        for l in links {
            t = t.link(l);
        }
        t = t.on_link(|s: &mut App, href: &str, tx: &mut Tx| s.open_link(href, tx));
    }
    t
}
