//! An event's notes as people read them. Invitations often carry HTML (Calendly's links) and
//! Google's block of joining instructions; the details show the words, and the call on its own.

/// The line Google opens and closes its joining instructions with.
const GOOGLE_MARK: &str = "-::~:~::~";

/// `description` as plain text: tags gone, entities decoded, Google's joining block left out.
pub fn clean(description: &str) -> String {
    let text = if looks_like_html(description) { html_text(description) } else { description.to_owned() };
    let text = match google_block(&text) {
        Some(block) => text.replacen(block, "", 1),
        None => text,
    };
    tidy(&text)
}

/// Google's joining instructions in `description`, from its opening line through its closing
/// one, kept when the notes are edited.
pub fn google_block(description: &str) -> Option<&str> {
    let start = description.find(GOOGLE_MARK)?;
    let line_start = description[..start].rfind('\n').map_or(0, |i| i + 1);
    let after = start + GOOGLE_MARK.len();
    let first_end = description[after..].find('\n').map_or(description.len(), |i| after + i);
    let end = match description[first_end..].find(GOOGLE_MARK) {
        Some(i) => {
            let close = first_end + i;
            description[close..].find('\n').map_or(description.len(), |j| close + j + 1)
        }
        None => description.len(),
    };
    Some(&description[line_start..end])
}

fn looks_like_html(s: &str) -> bool {
    ["<a ", "<br", "<p>", "<p ", "<div", "<b>", "<i>", "<ul", "<li", "<span", "</"].iter().any(|t| s.contains(t))
}

/// HTML to text: block tags break lines, links keep their words, entities decode.
fn html_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        match name {
            "br" => out.push('\n'),
            "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "ul" | "ol" if !out.ends_with('\n') && !out.is_empty() => out.push('\n'),
            _ => {}
        }
        if name == "li" && !tag.starts_with('/') {
            out.push_str("• ");
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    decode(&out)
}

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let end = tail.find(';').filter(|&i| i <= 10);
        let decoded = end.and_then(|i| match &tail[1..i] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            n => n.strip_prefix("#x").map_or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()), |h| u32::from_str_radix(h, 16).ok())
                .and_then(char::from_u32),
        });
        match (decoded, end) {
            (Some(c), Some(i)) => {
                out.push(c);
                rest = &tail[i + 1..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Trailing spaces off each line, at most one blank line in a row, none at either end.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.lines().map(str::trim_end) {
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

/// The addresses in `text`, as written.
pub fn links(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("http").filter_map(move |(i, _)| {
        let tail = &text[i..];
        if !(tail.starts_with("https://") || tail.starts_with("http://")) {
            return None;
        }
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'')).unwrap_or(tail.len());
        Some(tail[..end].trim_end_matches(['.', ',', ')', ';']))
    })
}

/// The first link in `text` to a video call service.
pub fn call_link(text: &str) -> Option<&str> {
    links(text).find(|l| service(l).is_some())
}

/// The video call service a link joins, by name.
pub fn service(link: &str) -> Option<&'static str> {
    let host = host(link)?;
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    Some(match () {
        _ if on("meet.google.com") => "Google Meet",
        _ if on("zoom.us") || on("zoomgov.com") => "Zoom",
        _ if on("teams.microsoft.com") || on("teams.live.com") => "Microsoft Teams",
        _ if on("webex.com") => "Webex",
        _ if on("facetime.apple.com") => "FaceTime",
        _ if on("whereby.com") => "Whereby",
        _ if on("around.co") => "Around",
        _ => return None,
    })
}

/// A link's host, without `www.`.
pub fn host(link: &str) -> Option<String> {
    let host = url::Url::parse(link.trim()).ok()?.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALENDLY: &str = "Event Name\nAccount Review W/ Senior Strategist\n\nWebsite: <a href=\"https://calendly.com/url?q=https%3A%2F%2Faxcargoinc.com%2F&amp;stage=1\">axcargoinc.com/</a>\n\nPowered by Calendly.com\n\n\n-::~:~::~:~::~:~::~:~::~:~::~:~::~-\nJoin with Google Meet: https://meet.google.com/unb-tovf-wnw\nOr dial: (US) +1 484-552-3064 PIN: 226294330#\n\nPlease do not edit this section.\n-::~:~::~:~::~:~::~:~::~:~::~:~::~-\n";

    #[test]
    fn notes_lose_html_and_the_joining_block() {
        assert_eq!(
            clean(CALENDLY),
            "Event Name\nAccount Review W/ Senior Strategist\n\nWebsite: axcargoinc.com/\n\nPowered by Calendly.com"
        );
        let block = google_block(CALENDLY).unwrap();
        assert!(block.starts_with("-::~") && block.ends_with("~-\n") && block.contains("Please do not edit"));
        assert_eq!(clean("a<br>b &lt;c&gt; &#8211; <ul><li>one</li><li>two</li></ul>"), "a\nb <c> –\n• one\n• two");
        assert_eq!(clean("Fish & chips"), "Fish & chips", "plain text keeps its ampersands");
    }

    #[test]
    fn finds_calls() {
        assert_eq!(call_link(CALENDLY), Some("https://meet.google.com/unb-tovf-wnw"));
        assert_eq!(call_link("Join: https://acme.zoom.us/j/123?pwd=x."), Some("https://acme.zoom.us/j/123?pwd=x"));
        assert_eq!(call_link("https://calendly.com/cancellations/1"), None);
        assert_eq!(service("https://teams.microsoft.com/l/meetup-join/abc"), Some("Microsoft Teams"));
        assert_eq!(host("https://www.example.com/a").as_deref(), Some("example.com"));
    }
}
