//! The browser extension's way in. Chromium starts this binary as a native-messaging host (it
//! passes the extension's origin as the only argument); the host passes each request on, over a
//! socket only this user can open, to the window, which answers from the open vault, and nothing
//! while it's locked.
//!
//! Every account open in the window answers, not only the one on screen. Requests and answers
//! are JSON:
//! - `{"op":"logins","url":…}`: `{"accounts":[{"id","email","host","open"}],"logins":[{"account",
//!   "id","name","username","code"}]}`: every kept account (`open`: unlocked), and the open
//!   ones' logins for that page's site (`code`: it has a verification code). With none open,
//!   `"locked":true` as well.
//! - `{"op":"fill","account":…,"id":…,"url":…}`: `{"username","password"}`, only if that login
//!   is for the page's site.
//! - `{"op":"code","account":…,"id":…,"url":…}`: `{"code"}`, the same way.
//!
//! Anything wrong comes back as `{"error":…}`. The host is `io.brem.rune_passwords` (its manifest
//! is written by scripts/install.sh; the extension is in extension/).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rune::Waker;
use serde_json::{Value, json};

use crate::app::AccountEntry;
use crate::vault::{Item, Kind, Vault};

/// A request from the browser, waiting on the window's answer.
pub struct Request {
    pub query: Value,
    pub reply: Sender<Value>,
}

fn socket() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from).join("rune-passwords.sock")
}

/// Listens for the host on a thread of its own; requests come out of the receiver, with `wake`
/// called for each. None if the socket can't be had (another window has it, say).
pub fn listen(wake: Waker) -> Option<Receiver<Request>> {
    let path = socket();
    // One left by a window that's gone answers nothing: take its place. One that answers is
    // another window's.
    if UnixStream::connect(&path).is_ok() {
        return None;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).ok()?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok()?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("bridge".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, wake) = (tx.clone(), wake.clone());
                std::thread::spawn(move || serve(stream, &tx, &wake));
            }
        })
        .ok()?;
    Some(rx)
}

/// One line of request in, one line of answer out.
fn serve(stream: UnixStream, to_app: &Sender<Request>, wake: &Waker) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let answer = match serde_json::from_str::<Value>(&line) {
        Ok(query) => {
            let (reply, back) = channel();
            if to_app.send(Request { query, reply }).is_err() {
                return;
            }
            wake();
            back.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| json!({"error": "Rune Passwords didn't answer"}))
        }
        Err(e) => json!({"error": format!("Not JSON: {e}")}),
    };
    let mut out = &stream;
    let _ = writeln!(out, "{answer}");
}

/// The window's answer to `q`: `accounts` are every kept account, `open` the open vaults.
pub fn answer(accounts: &[AccountEntry], open: &[(&AccountEntry, &Vault)], q: &Value) -> Value {
    let url = q["url"].as_str().unwrap_or_default();
    let Some(page) = host_of(url) else { return json!({"error": "Not a web page"}) };
    let logins = || {
        open.iter().flat_map(|(a, v)| v.items.iter().map(move |i| (*a, i))).filter(|(_, i)| {
            i.kind == Kind::Login && i.deleted.is_none() && for_site(i, &page)
        })
    };
    let one = || {
        let (account, id) = (q["account"].as_str()?, q["id"].as_str()?);
        logins().find(|(a, i)| a.id == account && i.id == id).map(|(_, i)| i)
    };
    match q["op"].as_str().unwrap_or_default() {
        "logins" => {
            let list: Vec<Value> = accounts
                .iter()
                .map(|a| json!({"id": a.id, "email": a.email, "host": a.host, "open": open.iter().any(|(o, _)| o.id == a.id)}))
                .collect();
            let mut found: Vec<(&AccountEntry, &Item)> = logins().collect();
            found.sort_by_key(|(a, i)| (i.name.to_lowercase(), a.email.clone()));
            let logins: Vec<Value> = found
                .iter()
                .map(|(a, i)| json!({"account": a.id, "id": i.id, "name": i.name, "username": i.username, "code": !i.totp.is_empty()}))
                .collect();
            let mut out = json!({"accounts": list, "logins": logins});
            if open.is_empty() {
                out["locked"] = json!(true);
            }
            out
        }
        "fill" => match one() {
            Some(i) => json!({"username": i.username, "password": i.password}),
            None => json!({"error": "No such login for this site"}),
        },
        "code" => match one().and_then(|i| crate::totp::Totp::parse(&i.totp)) {
            Some(t) => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                json!({"code": t.code(now)})
            }
            None => json!({"error": "No verification code for this login"}),
        },
        other => json!({"error": format!("Unknown request {other:?}")}),
    }
}

/// A page's or a saved website's host, without "www.".
fn host_of(uri: &str) -> Option<String> {
    let with_scheme = if uri.contains("://") { uri.to_owned() } else { format!("https://{uri}") };
    let u = url::Url::parse(&with_scheme).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_owned())
}

/// A login is for a page when one of its websites is the page's host or a domain above it
/// (example.com fills on accounts.example.com, not the other way round).
fn for_site(i: &Item, page: &str) -> bool {
    i.uris.iter().filter_map(|u| host_of(u)).any(|h| page == h || page.ends_with(&format!(".{h}")))
}

/// The native-messaging host: Chromium's messages (a 4-byte length, then JSON) from stdin to
/// the window, and its answers back the same way, until the browser closes the pipe.
pub fn host() {
    let (mut input, mut output) = (std::io::stdin().lock(), std::io::stdout().lock());
    loop {
        let mut len = [0u8; 4];
        if input.read_exact(&mut len).is_err() {
            return;
        }
        let mut msg = vec![0u8; u32::from_ne_bytes(len) as usize];
        if input.read_exact(&mut msg).is_err() {
            return;
        }
        let answer = ask(&msg).unwrap_or_else(|| json!({"error": "Open Rune Passwords to fill from it"}));
        let bytes = answer.to_string().into_bytes();
        let _ = output.write_all(&(bytes.len() as u32).to_ne_bytes());
        let _ = output.write_all(&bytes);
        let _ = output.flush();
    }
}

fn ask(msg: &[u8]) -> Option<Value> {
    let query: Value = serde_json::from_slice(msg).ok()?;
    let mut stream = UnixStream::connect(socket()).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    writeln!(stream, "{query}").ok()?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(id: &str, uri: &str) -> Item {
        let mut i = Item::new_login(None);
        i.id = id.into();
        i.name = id.into();
        i.username = format!("{id}@example.com");
        i.password = "secret".into();
        i.uris = vec![uri.into()];
        i
    }

    fn account(id: &str) -> AccountEntry {
        AccountEntry { id: id.into(), email: format!("me@{id}.com"), server: String::new(), host: format!("vault.{id}.com") }
    }

    #[test]
    fn only_the_sites_own_logins_from_every_open_account() {
        let (home, work, spare) = (account("home"), account("work"), account("spare"));
        let mut v = Vault::default();
        v.items = vec![login("a", "https://www.example.com/login"), login("b", "other.org")];
        let mut w = Vault::default();
        w.items = vec![login("c", "example.com")];
        let all = [home.clone(), work.clone(), spare];
        let open = [(&home, &v), (&work, &w)];
        let ask = |q: Value| answer(&all, &open, &q);
        let locked = answer(&all, &[], &json!({"op": "logins", "url": "https://example.com"}));
        assert_eq!(locked["locked"], true);
        assert_eq!(locked["accounts"].as_array().map(|a| a.len()), Some(3));
        let found = ask(json!({"op": "logins", "url": "https://accounts.example.com/x"}));
        let ids: Vec<(&str, &str)> = found["logins"].as_array().unwrap().iter().map(|l| (l["account"].as_str().unwrap(), l["id"].as_str().unwrap())).collect();
        assert_eq!(ids, [("home", "a"), ("work", "c")]);
        assert_eq!(found["accounts"][2]["open"], false, "a kept account not unlocked");
        assert_eq!(ask(json!({"op": "fill", "account": "work", "id": "c", "url": "https://example.com"}))["password"], "secret");
        assert!(ask(json!({"op": "fill", "account": "work", "id": "a", "url": "https://example.com"}))["error"].is_string(), "not another account's");
        assert!(ask(json!({"op": "fill", "account": "home", "id": "b", "url": "https://example.com"}))["error"].is_string(), "not another site's");
        assert!(ask(json!({"op": "fill", "account": "home", "id": "a", "url": "https://notexample.com"}))["error"].is_string());
        assert!(ask(json!({"op": "logins", "url": "chrome://settings"}))["error"].is_string());
    }
}
