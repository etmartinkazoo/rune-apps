//! A small blocking JMAP client (RFC 8620/8621), enough for Fastmail: mailboxes, message lists,
//! bodies, keyword and mailbox changes, and sending through EmailSubmission.

use serde_json::{Value, json};

use crate::html::Doc;

pub const SESSION_URL: &str = "https://api.fastmail.com/jmap/session";
const USING: [&str; 3] = [
    "urn:ietf:params:jmap:core",
    "urn:ietf:params:jmap:mail",
    "urn:ietf:params:jmap:submission",
];
const SUMMARY_PROPS: [&str; 10] =
    ["id", "threadId", "from", "to", "subject", "preview", "receivedAt", "keywords", "hasAttachment", "mailboxIds"];

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Addr {
    pub name: Option<String>,
    pub email: String,
}

impl Addr {
    pub fn display(&self) -> &str {
        self.name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&self.email)
    }
    fn list(v: &Value) -> Vec<Addr> {
        v.as_array().map_or_else(Vec::new, |a| {
            a.iter()
                .map(|x| Addr {
                    name: x["name"].as_str().map(str::to_owned),
                    email: x["email"].as_str().unwrap_or_default().to_owned(),
                })
                .collect()
        })
    }
    fn json(list: &[Addr]) -> Value {
        Value::Array(list.iter().map(|a| json!({ "name": a.name, "email": a.email })).collect())
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: u32,
    pub unread: u32,
    pub total: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Summary {
    /// The JMAP id, unique within its account.
    pub id: String,
    /// The account's key (its username) and `account:id`, unique across accounts.
    pub account: String,
    pub uid: String,
    pub thread_id: String,
    /// The conversation this message is listed under (`account:threadId`), or its own uid
    /// when conversations are off.
    pub thread_key: String,
    pub from: Vec<Addr>,
    pub to: Vec<Addr>,
    pub subject: String,
    pub preview: String,
    pub received_at: String,
    pub seen: bool,
    pub flagged: bool,
    pub has_attachment: bool,
    pub mailbox_ids: Vec<String>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Body {
    /// Plain text: the text part, or the HTML part converted. Replies quote it.
    pub text: String,
    pub html: Option<Doc>,
    /// The HTML `html` was parsed from, with `cid:` images resolved: what the store keeps.
    #[serde(skip)]
    pub html_src: Option<String>,
    /// Attachment files (not images shown inline).
    #[serde(default)]
    pub files: Vec<FileInfo>,
    /// The `blob:` sources of the message's own inline images. Mail HTML can name any blob,
    /// so only these load without the user allowing remote images. `None` on bodies kept from
    /// before this was recorded.
    #[serde(default)]
    pub inline: Option<Vec<String>>,
    pub cc: Vec<Addr>,
    pub reply_to: Vec<Addr>,
    pub message_id: Vec<String>,
    pub references: Vec<String>,
    pub unsubscribe: Option<crate::privacy::Unsubscribe>,
}

/// An attachment file of a received message.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// Where it is: the blob id over JMAP, the attachment's position over IMAP.
    pub part: String,
}

#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    /// The body as HTML too, when it has pasted images at their places.
    pub html: Option<String>,
    pub attachments: Vec<crate::attach::Outgoing>,
    /// The sender's name, over the account's default.
    pub from_name: Option<String>,
}

pub struct Client {
    token: String,
    upload_url: String,
    api_url: String,
    account: String,
    pub username: String,
    pub downloads: Downloads,
}

/// Fetches blobs (inline images) and remote images; cheap to clone onto image threads.
#[derive(Clone)]
pub struct Downloads {
    token: String,
    account: String,
    url: String,
}

const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
/// Larger than any server accepts (Fastmail's limit is 70 MB), so only a runaway download stops.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Requests to Fastmail. Each read and write gives up after a minute, so a server that stops
/// answering can't hold the worker for good; there's no limit on the whole, as a large
/// attachment takes a while.
fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(20))
            .timeout_read(std::time::Duration::from_secs(60))
            .timeout_write(std::time::Duration::from_secs(60))
            .build()
    })
}

/// All of `response`, or Err if it's over `limit` bytes rather than the start of it.
fn read_all(response: ureq::Response, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(response.into_reader(), limit + 1), &mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("It's over {} MB", limit / (1024 * 1024)));
    }
    Ok(bytes)
}

impl Downloads {
    /// Fetches only http(s) images, for accounts without a blob store.
    pub fn web() -> Downloads {
        Downloads { token: String::new(), account: String::new(), url: String::new() }
    }

    fn blob(&self, id: &str, mime: &str, name: &str) -> ureq::Request {
        let url = self
            .url
            .replace("{accountId}", &self.account)
            .replace("{blobId}", id)
            .replace("{type}", &mime.replace('/', "%2F"))
            .replace("{name}", name);
        agent().get(&url).set("Authorization", &format!("Bearer {}", self.token))
    }

    /// An image: `blob:<id>:<type>` from the account, or an http(s) URL.
    pub fn fetch(&self, src: &str) -> Result<Vec<u8>> {
        let request = if let Some(rest) = src.strip_prefix("blob:") {
            let (id, mime) = rest.split_once(':').unwrap_or((rest, "application/octet-stream"));
            self.blob(id, mime, "image")
        } else {
            agent().get(src).set("User-Agent", crate::privacy::USER_AGENT)
        };
        let response = request.timeout(std::time::Duration::from_secs(20)).call().map_err(|e| e.to_string())?;
        read_all(response, MAX_IMAGE_BYTES)
    }

    /// An attachment's blob, whole.
    pub fn file(&self, blob: &str, mime: &str) -> Result<Vec<u8>> {
        let response = self.blob(blob, mime, "attachment").call().map_err(|e| e.to_string())?;
        read_all(response, MAX_FILE_BYTES)
    }
}

impl Client {
    pub fn connect(token: &str) -> Result<Client> {
        let session: Value = agent()
            .get(SESSION_URL)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => "Fastmail rejected the API token.".to_owned(),
                e => format!("Couldn't reach Fastmail: {e}"),
            })?
            .into_json()
            .map_err(|e| e.to_string())?;
        let account = session["primaryAccounts"]["urn:ietf:params:jmap:mail"]
            .as_str()
            .ok_or("The token has no mail access. Give it Email scope.")?;
        Ok(Client {
            token: token.to_owned(),
            api_url: session["apiUrl"].as_str().ok_or("Session has no apiUrl")?.to_owned(),
            upload_url: session["uploadUrl"].as_str().unwrap_or_default().to_owned(),
            account: account.to_owned(),
            username: session["username"].as_str().unwrap_or_default().to_owned(),
            downloads: Downloads {
                token: token.to_owned(),
                account: account.to_owned(),
                url: session["downloadUrl"].as_str().unwrap_or_default().to_owned(),
            },
        })
    }

    /// Sends method calls (named `c0`, `c1`, …) and returns each response's arguments in order.
    fn call(&self, calls: Vec<(&str, Value)>) -> Result<Vec<Value>> {
        let method_calls: Vec<Value> =
            calls.into_iter().enumerate().map(|(i, (name, args))| json!([name, args, format!("c{i}")])).collect();
        let calls = method_calls.len();
        let response: Value = agent()
            .post(&self.api_url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .send_json(json!({ "using": USING, "methodCalls": method_calls }))
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in response["methodResponses"].as_array().ok_or("Malformed JMAP response")? {
            if r[0] == "error" {
                return Err(format!("JMAP error: {}", r[1]["type"].as_str().unwrap_or("unknown")));
            }
            for kind in ["notCreated", "notUpdated"] {
                if let Some((_, err)) = r[1][kind].as_object().and_then(|m| m.iter().next()) {
                    let detail = err["description"].as_str().or(err["type"].as_str()).unwrap_or("unknown");
                    return Err(format!("{}: {detail}", r[0].as_str().unwrap_or_default()));
                }
            }
            out.push(r[1].clone());
        }
        // Callers index the answers by call; a server that answered fewer mustn't panic them.
        if out.len() < calls {
            return Err(format!("JMAP answered {} of {calls} calls", out.len()));
        }
        Ok(out)
    }

    /// The account's Email state: it changes when any message arrives, goes, or changes.
    pub fn email_state(&self) -> Result<String> {
        let r = self.call(vec![("Email/get", json!({ "accountId": self.account, "ids": [], "properties": ["id"] }))])?;
        r[0]["state"].as_str().map(str::to_owned).ok_or_else(|| "No Email state".to_owned())
    }

    pub fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let r = self.call(vec![(
            "Mailbox/get",
            json!({ "accountId": self.account, "ids": null,
                    "properties": ["name", "role", "parentId", "sortOrder", "unreadEmails", "totalEmails"] }),
        )])?;
        let list = r[0]["list"].as_array().cloned().unwrap_or_default();
        Ok(list
            .iter()
            .map(|m| Mailbox {
                id: str_of(&m["id"]),
                name: str_of(&m["name"]),
                role: m["role"].as_str().map(str::to_owned),
                parent_id: m["parentId"].as_str().map(str::to_owned),
                sort_order: count(&m["sortOrder"]),
                unread: count(&m["unreadEmails"]),
                total: count(&m["totalEmails"]),
            })
            .collect())
    }

    /// The newest `limit` messages in a mailbox, optionally matching a search.
    pub fn messages(&self, mailbox: &str, search: &str, limit: usize) -> Result<Vec<Summary>> {
        let mut filter = json!({ "inMailbox": mailbox });
        if !search.trim().is_empty() {
            filter["text"] = json!(search.trim());
        }
        let r = self.call(vec![
            (
                "Email/query",
                json!({ "accountId": self.account, "filter": filter, "limit": limit,
                        "sort": [{ "property": "receivedAt", "isAscending": false }] }),
            ),
            (
                "Email/get",
                json!({ "accountId": self.account, "properties": SUMMARY_PROPS,
                        "#ids": { "resultOf": "c0", "name": "Email/query", "path": "/ids" } }),
            ),
        ])?;
        let list = r[1]["list"].as_array().cloned().unwrap_or_default();
        Ok(list.iter().map(|m| summary(m, &self.username)).collect())
    }

    /// The newest `limit` conversations in a mailbox: the newest message of each in the
    /// mailbox (in list order), and every message of those conversations, wherever it is.
    pub fn conversations(&self, mailbox: &str, search: &str, limit: usize) -> Result<(Vec<String>, Vec<Summary>)> {
        let mut filter = json!({ "inMailbox": mailbox });
        if !search.trim().is_empty() {
            filter["text"] = json!(search.trim());
        }
        let r = self.call(vec![
            (
                "Email/query",
                json!({ "accountId": self.account, "filter": filter, "limit": limit, "collapseThreads": true,
                        "sort": [{ "property": "receivedAt", "isAscending": false }] }),
            ),
            (
                "Email/get",
                json!({ "accountId": self.account, "properties": ["threadId"],
                        "#ids": { "resultOf": "c0", "name": "Email/query", "path": "/ids" } }),
            ),
            (
                "Thread/get",
                json!({ "accountId": self.account,
                        "#ids": { "resultOf": "c1", "name": "Email/get", "path": "/list/*/threadId" } }),
            ),
            (
                "Email/get",
                json!({ "accountId": self.account, "properties": SUMMARY_PROPS,
                        "#ids": { "resultOf": "c2", "name": "Thread/get", "path": "/list/*/emailIds" } }),
            ),
        ])?;
        let newest = strings(&r[0]["ids"]);
        let list = r[3]["list"].as_array().cloned().unwrap_or_default();
        Ok((newest, list.iter().map(|m| summary(m, &self.username)).collect()))
    }

    pub fn body(&self, id: &str) -> Result<Body> {
        let r = self.call(vec![(
            "Email/get",
            json!({ "accountId": self.account, "ids": [id],
                    "properties": ["textBody", "htmlBody", "bodyValues", "attachments", "cc", "replyTo",
                                   "messageId", "references", "header:List-Unsubscribe:asURLs",
                                   "header:List-Unsubscribe-Post:asText", "header:List-Id:asText"],
                    "fetchAllBodyValues": true, "maxBodyValueBytes": 2_000_000 }),
        )])?;
        let m = &r[0]["list"][0];
        let value = |part: &Value| m["bodyValues"][str_of(&part["partId"])]["value"].as_str().unwrap_or_default().to_owned();
        let mut text = String::new();
        for part in m["textBody"].as_array().into_iter().flatten() {
            let v = value(part);
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            if part["type"] == "text/html" {
                text.push_str(&crate::html::to_text(&v));
            } else {
                text.push_str(&v);
            }
        }
        let html_parts: Vec<String> =
            m["htmlBody"].as_array().into_iter().flatten().filter(|p| p["type"] == "text/html").map(value).collect();
        let atts: Vec<Value> = m["attachments"].as_array().cloned().unwrap_or_default();
        let cids: Vec<(String, String)> = atts
            .iter()
            .filter_map(|a| {
                let cid = a["cid"].as_str()?.trim_matches(['<', '>']).to_owned();
                Some((cid, format!("blob:{}:{}", a["blobId"].as_str()?, a["type"].as_str().unwrap_or("image/png"))))
            })
            .collect();
        let files: Vec<FileInfo> = atts
            .iter()
            .filter(|a| a["cid"].is_null() || a["disposition"] == "attachment")
            .filter_map(|a| {
                Some(FileInfo {
                    name: a["name"].as_str().unwrap_or("attachment").to_owned(),
                    mime: a["type"].as_str().unwrap_or("application/octet-stream").to_owned(),
                    size: a["size"].as_u64().unwrap_or(0),
                    part: a["blobId"].as_str()?.to_owned(),
                })
            })
            .collect();
        let html_src = (!html_parts.is_empty()).then(|| {
            let resolve = |id: &str| cids.iter().find(|(c, _)| c == id.trim_matches(['<', '>'])).map(|(_, s)| s.clone());
            crate::html::resolve_cids(&html_parts.join("\n"), &resolve)
        });
        let html = html_src.as_deref().map(|h| crate::html::parse(h, &|_| None));
        Ok(Body {
            text: text.trim_end().to_owned(),
            html,
            html_src,
            files,
            inline: Some(cids.into_iter().map(|(_, src)| src).collect()),
            cc: Addr::list(&m["cc"]),
            reply_to: Addr::list(&m["replyTo"]),
            message_id: strings(&m["messageId"]),
            references: strings(&m["references"]),
            unsubscribe: {
                let urls: Vec<String> = strings(&m["header:List-Unsubscribe:asURLs"]).iter().map(|u| format!("<{u}>")).collect();
                crate::privacy::Unsubscribe::parse(
                    &urls.join(", "),
                    m["header:List-Unsubscribe-Post:asText"].as_str(),
                    m["header:List-Id:asText"].as_str(),
                )
            },
        })
    }

    /// Sets (`true`) or clears a keyword such as `$seen` or `$flagged`.
    pub fn set_keyword(&self, ids: &[String], keyword: &str, on: bool) -> Result<()> {
        let patch = json!({ format!("keywords/{keyword}"): if on { json!(true) } else { Value::Null } });
        let update: serde_json::Map<String, Value> = ids.iter().map(|id| (id.clone(), patch.clone())).collect();
        self.call(vec![("Email/set", json!({ "accountId": self.account, "update": update }))]).map(drop)
    }

    pub fn move_to(&self, ids: &[String], from: &str, to: &str) -> Result<()> {
        let patch = json!({ format!("mailboxIds/{from}"): null, format!("mailboxIds/{to}"): true });
        let update: serde_json::Map<String, Value> = ids.iter().map(|id| (id.clone(), patch.clone())).collect();
        self.call(vec![("Email/set", json!({ "accountId": self.account, "update": update }))]).map(drop)
    }

    /// Uploads bytes as a blob for an attachment; returns its id.
    fn upload(&self, mime: &str, bytes: &[u8]) -> Result<String> {
        let url = self.upload_url.replace("{accountId}", &self.account);
        let r: Value = agent()
            .post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", mime)
            .send_bytes(bytes)
            .map_err(|e| format!("Uploading an attachment: {e}"))?
            .into_json()
            .map_err(|e| e.to_string())?;
        r["blobId"].as_str().map(str::to_owned).ok_or_else(|| "The upload gave no blob".to_owned())
    }

    pub fn send(&self, mail: &Outgoing, drafts: &str, sent: &str) -> Result<()> {
        let r = self.call(vec![("Identity/get", json!({ "accountId": self.account, "ids": null }))])?;
        let identities = r[0]["list"].as_array().cloned().unwrap_or_default();
        let identity = identities
            .iter()
            .find(|i| i["email"].as_str() == Some(&self.username))
            .or(identities.first())
            .ok_or("No sending identity on this account")?;
        let name = mail.from_name.clone().or_else(|| identity["name"].as_str().map(str::to_owned)).filter(|n| !n.is_empty());
        let from = Addr { name, email: str_of(&identity["email"]) };
        let mut email = json!({
            "mailboxIds": { drafts: true },
            "keywords": { "$draft": true, "$seen": true },
            "from": Addr::json(&[from]),
            "to": Addr::json(&mail.to),
            "subject": mail.subject,
            "textBody": [{ "partId": "body", "type": "text/plain" }],
            "bodyValues": { "body": { "value": mail.body } },
        });
        if let Some(html) = &mail.html {
            email["htmlBody"] = json!([{ "partId": "html", "type": "text/html" }]);
            email["bodyValues"]["html"] = json!({ "value": html });
        }
        if !mail.attachments.is_empty() {
            let mut parts = Vec::new();
            for a in &mail.attachments {
                let blob = self.upload(&a.mime, &a.bytes)?;
                // Inline images (a cid) go in multipart/related with the HTML; files attach.
                parts.push(match &a.cid {
                    Some(cid) => json!({ "blobId": blob, "type": a.mime, "name": a.name, "cid": cid, "disposition": "inline" }),
                    None => json!({ "blobId": blob, "type": a.mime, "name": a.name, "disposition": "attachment" }),
                });
            }
            email["attachments"] = Value::Array(parts);
        }
        if !mail.cc.is_empty() {
            email["cc"] = Addr::json(&mail.cc);
        }
        if !mail.in_reply_to.is_empty() {
            email["inReplyTo"] = json!(mail.in_reply_to);
            email["references"] = json!(mail.references);
        }
        self.call(vec![
            ("Email/set", json!({ "accountId": self.account, "create": { "draft": email } })),
            (
                "EmailSubmission/set",
                json!({ "accountId": self.account,
                        "create": { "send": { "identityId": identity["id"], "emailId": "#draft" } },
                        "onSuccessUpdateEmail": { "#send": {
                            format!("mailboxIds/{drafts}"): null,
                            format!("mailboxIds/{sent}"): true,
                            "keywords/$draft": null } } }),
            ),
        ])
        .map(drop)
    }
}

pub fn uid(account: &str, id: &str) -> String {
    format!("{account}:{id}")
}

fn summary(m: &Value, account: &str) -> Summary {
    let keywords = &m["keywords"];
    Summary {
        id: str_of(&m["id"]),
        account: account.to_owned(),
        uid: uid(account, &str_of(&m["id"])),
        thread_id: str_of(&m["threadId"]),
        thread_key: uid(account, &str_of(&m["id"])),
        from: Addr::list(&m["from"]),
        to: Addr::list(&m["to"]),
        subject: str_of(&m["subject"]),
        preview: str_of(&m["preview"]),
        received_at: str_of(&m["receivedAt"]),
        seen: keywords["$seen"] == true,
        flagged: keywords["$flagged"] == true,
        has_attachment: m["hasAttachment"] == true,
        mailbox_ids: m["mailboxIds"].as_object().map_or_else(Vec::new, |o| o.keys().cloned().collect()),
    }
}

/// A count from the server, as large as fits.
fn count(v: &Value) -> u32 {
    v.as_u64().map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map_or_else(Vec::new, |a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
}

/// Parses "Name <a@b>, c@d" into addresses.
pub fn parse_addrs(s: &str) -> Vec<Addr> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match (p.rfind('<'), p.rfind('>')) {
            (Some(l), Some(r)) if l < r => {
                let name = p[..l].trim().trim_matches('"').trim();
                Addr { name: (!name.is_empty()).then(|| name.to_owned()), email: p[l + 1..r].trim().to_owned() }
            }
            _ => Addr { name: None, email: p.to_owned() },
        })
        .collect()
}

pub fn format_addrs(list: &[Addr]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
            _ => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_bare_addresses() {
        let a = parse_addrs("\"Ada Lovelace\" <ada@example.com>, bob@example.com");
        assert_eq!(a[0], Addr { name: Some("Ada Lovelace".into()), email: "ada@example.com".into() });
        assert_eq!(a[1], Addr { name: None, email: "bob@example.com".into() });
        assert_eq!(format_addrs(&a), "Ada Lovelace <ada@example.com>, bob@example.com");
    }

    /// A one-request HTTP server on localhost answering `body`; its URL.
    fn serve(body: &'static str) -> String {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(c.try_clone().unwrap());
            let mut line = String::new();
            let mut length = 0;
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap_or(0);
                }
                line.clear();
            }
            std::io::Read::read_exact(&mut reader, &mut vec![0; length]).unwrap();
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            c.write_all(head.as_bytes()).and_then(|()| c.write_all(body.as_bytes())).unwrap();
        });
        url
    }

    #[test]
    fn a_download_over_its_limit_fails_rather_than_coming_back_cut_short() {
        let small = agent().get(&serve("0123456789")).call().unwrap();
        assert_eq!(read_all(small, 10).unwrap(), b"0123456789");
        let big = agent().get(&serve("0123456789+")).call().unwrap();
        assert!(read_all(big, 10).is_err());
    }

    #[test]
    fn a_short_answer_is_an_error_not_a_panic() {
        let client = Client {
            token: String::new(),
            upload_url: String::new(),
            api_url: serve(r#"{"methodResponses":[["Email/query",{"ids":[]},"c0"]]}"#),
            account: "a".into(),
            username: "a@x.com".into(),
            downloads: Downloads::web(),
        };
        assert!(client.messages("in", "", 10).is_err());
    }
}
