//! Tracker blocking and unsubscribing.
//!
//! Senders learn you opened a message from images fetched off their servers: a 1×1 "spy
//! pixel", or any picture with a per-recipient URL. Remote images stay off until you load
//! them; even then, images that are tracking pixels or come from known open-tracking
//! endpoints never load. Links are cleaned when clicked: redirect wrappers are unwrapped and
//! analytics parameters (`utm_*`, `fbclid`, …) removed. Click trackers that encode the
//! destination can't be unwrapped; they're counted so the reader can say so.
//!
//! This is pattern matching against the common mail-tracking services, not a guarantee: a
//! sender can always track loaded images from its own domain with an innocent-looking URL,
//! which is why remote content stays off by default.

/// Open-tracking endpoints: (host suffix, path fragment, service). An empty fragment
/// matches any path on that host.
const OPEN_TRACKERS: &[(&str, &str, &str)] = &[
    ("list-manage.com", "/track/open", "Mailchimp"),
    ("mailchimp.com", "/track/open", "Mailchimp"),
    ("sendgrid.net", "/wf/open", "SendGrid"),
    ("ct.sendgrid.net", "", "SendGrid"),
    ("mandrillapp.com", "/track/open", "Mandrill"),
    ("mailgun.org", "/o/", "Mailgun"),
    ("pstmrk.it", "/open", "Postmark"),
    ("hubspotemail.net", "", "HubSpot"),
    ("hubspotlinks.com", "", "HubSpot"),
    ("track.hubspot.com", "", "HubSpot"),
    ("exct.net", "/open", "Salesforce Marketing Cloud"),
    ("rs6.net", "/on.jsp", "Constant Contact"),
    ("createsend1.com", "/t/", "Campaign Monitor"),
    ("cmail19.com", "/t/", "Campaign Monitor"),
    ("cmail20.com", "/t/", "Campaign Monitor"),
    ("customeriomail.com", "/e/o/", "Customer.io"),
    ("klaviyomail.com", "", "Klaviyo"),
    ("convertkit-mail.com", "/open", "ConvertKit"),
    ("convertkit-mail2.com", "/open", "ConvertKit"),
    ("sendibt3.com", "", "Brevo"),
    ("sibautomation.com", "", "Brevo"),
    ("mailtrack.io", "", "Mailtrack"),
    ("mailtrack.com", "", "Mailtrack"),
    ("yesware.com", "", "Yesware"),
    ("getnotify.com", "", "Notify"),
    ("bananatag.com", "", "Bananatag"),
    ("mailfoogae.appspot.com", "", "Streak"),
    ("r.superhuman.com", "", "Superhuman"),
    ("track.mixmax.com", "", "Mixmax"),
    ("via.intercom.io", "", "Intercom"),
    ("intercom-mail.com", "", "Intercom"),
    ("spgo.io", "", "SparkPost"),
    ("emltrk.com", "", "Litmus"),
    ("litmus.com", "/pixel", "Litmus"),
    ("google-analytics.com", "/collect", "Google Analytics"),
    ("facebook.com", "/tr", "Facebook"),
    ("linkedin.com", "/emimp/", "LinkedIn"),
    ("substack.com", "/o/", "Substack"),
    ("amazonses.com", "/open", "Amazon SES"),
    ("awstrack.me", "", "Amazon SES"),
];

/// Click-tracking redirectors whose destination is encoded, so it can't be recovered.
const CLICK_TRACKERS: &[(&str, &str)] = &[
    ("list-manage.com", "Mailchimp"),
    ("sendgrid.net", "SendGrid"),
    ("mandrillapp.com", "Mandrill"),
    ("mailgun.org", "Mailgun"),
    ("hubspotlinks.com", "HubSpot"),
    ("hs-sites.com", "HubSpot"),
    ("exct.net", "Salesforce Marketing Cloud"),
    ("rs6.net", "Constant Contact"),
    ("createsend1.com", "Campaign Monitor"),
    ("customeriomail.com", "Customer.io"),
    ("klaviyomail.com", "Klaviyo"),
    ("convertkit-mail.com", "ConvertKit"),
    ("convertkit-mail2.com", "ConvertKit"),
    ("awstrack.me", "Amazon SES"),
    ("pstmrk.it", "Postmark"),
    ("spgo.io", "SparkPost"),
    ("substack.com/redirect", "Substack"),
    ("mailtrack.io", "Mailtrack"),
];

/// Path words that mark an image as an open beacon on any host.
const BEACON_WORDS: &[&str] = &["/open.php", "/open.gif", "/wf/open", "/track/open", "/beacon", "/pixel.gif", "/pixel.png", "/e/o/", "/o.gif", "/trk/", "/imp?"];

/// Query parameters that only exist for analytics.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "dclid", "msclkid", "yclid", "mc_cid", "mc_eid", "_hsenc", "_hsmi", "mkt_tok", "vero_id",
    "vero_conv", "oly_anon_id", "oly_enc_id", "rb_clickid", "s_cid", "ml_subscriber", "ml_subscriber_hash", "trk",
    "trkcampaign", "sc_campaign", "sc_channel", "igshid", "wickedid", "__s", "ck_subscriber_id", "_ke", "_kx",
];

/// Wrappers that carry the real link in a parameter: (host suffix, parameter).
const WRAPPERS: &[(&str, &str)] = &[
    ("google.com", "q"),
    ("google.com", "url"),
    ("safelinks.protection.outlook.com", "url"),
    ("l.facebook.com", "u"),
    ("l.instagram.com", "u"),
    ("t.umblr.com", "z"),
    ("out.reddit.com", "url"),
    ("youtube.com", "q"),
    ("slack-redir.net", "url"),
];

fn host_matches(host: &str, suffix: &str) -> bool {
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

/// Whether an image is a tracker, and whose.
pub fn image_tracker(src: &str, width: Option<f32>, height: Option<f32>) -> Option<&'static str> {
    if width.is_some_and(|w| w <= 3.0) || height.is_some_and(|h| h <= 3.0) {
        return Some("Tracking pixel");
    }
    let url = url::Url::parse(src).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let path = format!("{}{}", url.path(), url.query().map(|q| format!("?{q}")).unwrap_or_default()).to_ascii_lowercase();
    for (suffix, fragment, name) in OPEN_TRACKERS {
        if host_matches(&host, suffix) && (fragment.is_empty() || path.contains(fragment)) {
            return Some(name);
        }
    }
    BEACON_WORDS.iter().any(|w| path.contains(w)).then_some("Tracking pixel")
}

/// The service behind a click-tracking link, if it's one that can't be unwrapped.
pub fn click_tracker(href: &str) -> Option<&'static str> {
    let url = url::Url::parse(href).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let at = format!("{host}{}", url.path());
    CLICK_TRACKERS.iter().find(|(s, _)| if s.contains('/') { at.contains(s) } else { host_matches(&host, s) }).map(|(_, n)| *n)
}

/// A link with redirect wrappers unwrapped and analytics parameters removed.
pub fn clean_link(href: &str) -> String {
    let Ok(mut url) = url::Url::parse(href) else { return href.to_owned() };
    for _ in 0..3 {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else { break };
        let inner = WRAPPERS
            .iter()
            .filter(|(s, _)| host_matches(&host, s))
            .find_map(|(_, p)| url.query_pairs().find(|(k, _)| k == p).map(|(_, v)| v.into_owned()))
            .filter(|v| v.starts_with("http://") || v.starts_with("https://"));
        match inner.and_then(|v| url::Url::parse(&v).ok()) {
            Some(next) => url = next,
            None => break,
        }
    }
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            !k.starts_with("utm_") && !TRACKING_PARAMS.contains(&k.as_str())
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if url.query().is_some() {
        if kept.is_empty() {
            url.set_query(None);
        } else {
            url.query_pairs_mut().clear().extend_pairs(kept);
        }
    }
    url.to_string()
}

/// How a mailing list says to unsubscribe (List-Unsubscribe, RFC 2369; one-click, RFC 8058).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Unsubscribe {
    /// An https URL that unsubscribes with one POST, no page to visit.
    pub one_click: Option<String>,
    /// A mailto: address (with its subject and body, if given).
    pub mailto: Option<String>,
    /// A web page to visit.
    pub web: Option<String>,
    /// The list's name, from List-Id.
    pub list: Option<String>,
}

impl Unsubscribe {
    /// From the header values: List-Unsubscribe's `<url>, <mailto:…>`, whether
    /// List-Unsubscribe-Post asks for one-click, and List-Id.
    pub fn parse(header: &str, post: Option<&str>, list_id: Option<&str>) -> Option<Unsubscribe> {
        // Each URL is in angle brackets, which may hold commas of their own.
        let urls: Vec<&str> = header
            .split('<')
            .skip(1)
            .filter_map(|p| p.split_once('>').map(|(url, _)| url.trim()))
            .collect();
        let web = urls.iter().find(|u| u.starts_with("https://") || u.starts_with("http://")).map(|u| u.to_string());
        let mailto = urls.iter().find(|u| u.to_ascii_lowercase().starts_with("mailto:")).map(|u| u.to_string());
        let one_click = post
            .filter(|p| p.replace(' ', "").eq_ignore_ascii_case("List-Unsubscribe=One-Click"))
            .and(web.clone())
            .filter(|u| u.starts_with("https://"));
        if web.is_none() && mailto.is_none() {
            return None;
        }
        let list = list_id.map(|l| {
            let name = l.split('<').next().unwrap_or(l).trim().trim_matches('"').trim();
            if name.is_empty() { l.trim().trim_matches(['<', '>']).to_owned() } else { name.to_owned() }
        });
        Some(Unsubscribe { one_click, mailto, web, list })
    }

    /// The way it will happen, for the confirmation: one-click and mail need no page. Mail
    /// names where it goes, since it's sent from your account.
    pub fn method(&self) -> String {
        if self.one_click.is_some() {
            "one click, without opening a page".into()
        } else if let Some(url) = &self.mailto {
            format!("sending an unsubscribe email to {}", mailto(url).0)
        } else {
            "opening the sender's unsubscribe page".into()
        }
    }
}

/// A mailto: URL's address, subject and body.
pub fn mailto(url: &str) -> (String, String, String) {
    let rest = &url[url.find(':').map_or(0, |i| i + 1)..];
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let param = |name: &str| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.into_owned())
    };
    let addr = url::form_urlencoded::parse(addr.as_bytes()).map(|(k, _)| k.into_owned()).next().unwrap_or_default();
    // One address: the list's. A sender listing more would have mail sent for it from your
    // account to wherever it liked.
    let addr = addr.split(',').next().unwrap_or_default().trim().to_owned();
    (addr, param("subject").unwrap_or_else(|| "unsubscribe".into()), param("body").unwrap_or_else(|| "unsubscribe".into()))
}

/// The request one-click unsubscribing sends (RFC 8058 §3.1).
pub fn one_click(url: &str) -> Result<(), String> {
    ureq::post(url)
        .set("User-Agent", USER_AGENT)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .timeout(std::time::Duration::from_secs(20))
        .send_string("List-Unsubscribe=One-Click")
        .map(drop)
        .map_err(|e| format!("Unsubscribing failed: {e}"))
}

/// What image and unsubscribe requests call themselves: a common browser's string, so
/// requests don't single out this app.
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_pixels_and_open_trackers_but_not_content_images() {
        assert_eq!(image_tracker("https://x.example/a.png", Some(1.0), Some(1.0)), Some("Tracking pixel"));
        assert_eq!(image_tracker("https://acme.us1.list-manage.com/track/open.php?u=1", None, None), Some("Mailchimp"));
        assert_eq!(image_tracker("https://u123.ct.sendgrid.net/wf/open?upn=abc", None, None), Some("SendGrid"));
        assert_eq!(image_tracker("https://news.example.com/beacon?id=9", None, None), Some("Tracking pixel"));
        assert_eq!(image_tracker("https://mcusercontent.com/logo.png", Some(200.0), None), None);
        assert_eq!(image_tracker("https://cdn.example.com/hero.jpg", None, None), None);
    }

    #[test]
    fn cleans_links() {
        assert_eq!(
            clean_link("https://shop.example/p?id=4&utm_source=news&utm_medium=email&fbclid=x"),
            "https://shop.example/p?id=4"
        );
        assert_eq!(
            clean_link("https://www.google.com/url?q=https://site.example/a?utm_campaign=x&sa=D"),
            "https://site.example/a"
        );
        assert_eq!(
            clean_link("https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fdocs.example%2Fx&data=1"),
            "https://docs.example/x"
        );
        assert_eq!(clean_link("https://plain.example/"), "https://plain.example/");
        assert_eq!(click_tracker("https://acme.us1.list-manage.com/track/click?u=1"), Some("Mailchimp"));
        assert_eq!(click_tracker("https://example.com/a"), None);
    }

    #[test]
    fn parses_list_unsubscribe() {
        let u = Unsubscribe::parse(
            "<mailto:leave@list.example?subject=unsub>, <https://list.example/u/123>",
            Some("List-Unsubscribe=One-Click"),
            Some("Weekly News <news.list.example>"),
        )
        .unwrap();
        assert_eq!(u.one_click.as_deref(), Some("https://list.example/u/123"));
        assert_eq!(u.mailto.as_deref(), Some("mailto:leave@list.example?subject=unsub"));
        assert_eq!(u.list.as_deref(), Some("Weekly News"));
        assert_eq!(mailto(u.mailto.as_deref().unwrap()), ("leave@list.example".into(), "unsub".into(), "unsubscribe".into()));
        let web_only = Unsubscribe::parse("<http://list.example/u>", None, None).unwrap();
        assert_eq!(web_only.one_click, None, "one-click needs the Post header and https");
        assert!(Unsubscribe::parse("garbage", None, None).is_none());
        let comma = Unsubscribe::parse("<https://list.example/u?ids=1,2>, <mailto:u@list.example>", None, None).unwrap();
        assert_eq!(comma.web.as_deref(), Some("https://list.example/u?ids=1,2"), "a comma inside the brackets stays");
        assert_eq!(mailto("mailto:u@list.example,victim@example.com?subject=x").0, "u@list.example");
        assert_eq!(comma.method(), "sending an unsubscribe email to u@list.example", "the confirmation says where mail goes");
    }
}
