//! Vaultwarden (and Bitwarden) over HTTPS: the identity endpoints for signing in and renewing
//! the session, and the API for the vault. Everything sent is already encrypted, apart from
//! the master password's hash.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::crypto::{Kdf, Result};
use crate::vault::field;

/// Requests give up after this long without an answer, so a server that's gone doesn't hang
/// the app.
fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(60))
            .user_agent("Rune Passwords")
            .build()
    })
}

/// The device this app signs in as: Bitwarden's "Linux desktop".
const DEVICE_TYPE: &str = "8";
const CLIENT_ID: &str = "desktop";
/// The official client release whose API this speaks; servers check it to know what a client
/// sends (`encryptedFor` on items, from 2025.6).
const CLIENT_VERSION: &str = "2025.10.0";

/// A request with the headers servers expect of a Bitwarden client.
fn request(method: &str, url: &str) -> ureq::Request {
    agent()
        .request(method, url)
        .set("Bitwarden-Client-Name", CLIENT_ID)
        .set("Bitwarden-Client-Version", CLIENT_VERSION)
        .set("Device-Type", DEVICE_TYPE)
}

/// The server's base address, as typed (`https://vault.example.com`), without a trailing
/// slash. Bitwarden's own cloud splits identity and API onto hosts of their own.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Server(pub String);

impl Server {
    pub fn new(typed: &str) -> Result<Server> {
        let t = typed.trim().trim_end_matches('/');
        let t = if t.contains("://") { t.to_owned() } else { format!("https://{t}") };
        let url = url::Url::parse(&t).map_err(|_| "That isn't a server address".to_owned())?;
        if url.scheme() != "https" && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")) {
            return Err("The server needs to use HTTPS".into());
        }
        Ok(Server(t))
    }

    fn identity(&self, path: &str) -> String {
        match self.0.as_str() {
            "https://vault.bitwarden.com" => format!("https://identity.bitwarden.com{path}"),
            "https://vault.bitwarden.eu" => format!("https://identity.bitwarden.eu{path}"),
            base => format!("{base}/identity{path}"),
        }
    }

    fn api(&self, path: &str) -> String {
        match self.0.as_str() {
            "https://vault.bitwarden.com" => format!("https://api.bitwarden.com{path}"),
            "https://vault.bitwarden.eu" => format!("https://api.bitwarden.eu{path}"),
            base => format!("{base}/api{path}"),
        }
    }
}

/// An error the server explained, or the network's.
fn error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, response) => {
            let body: Value = response.into_json().unwrap_or(Value::Null);
            let said = [
                field(field(&body, "errorModel"), "message"),
                field(&body, "message"),
                field(&body, "error_description"),
            ]
            .into_iter()
            .find_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned));
            said.unwrap_or_else(|| format!("The server answered {code}"))
        }
        ureq::Error::Transport(t) => format!("Couldn't reach the server: {t}"),
    }
}

/// How the account derives its key, and what it's salted with.
pub fn prelogin(server: &Server, email: &str) -> Result<(Kdf, String)> {
    let r: Value = request("POST", &server.identity("/accounts/prelogin"))
        .send_json(json!({ "email": email.trim() }))
        .map_err(error)?
        .into_json()
        .map_err(|e| e.to_string())?;
    let kdf = Kdf::from_wire(
        field(&r, "kdf").as_u64().unwrap_or(0),
        field(&r, "kdfIterations").as_u64().unwrap_or(600_000),
        field(&r, "kdfMemory").as_u64(),
        field(&r, "kdfParallelism").as_u64(),
    )?;
    let salt = field(&r, "salt").as_str().filter(|s| !s.is_empty()).unwrap_or(email).to_owned();
    Ok((kdf, salt))
}

/// What signing in gives: the session, and the account's keys as the server keeps them.
#[derive(Clone, Debug)]
pub struct Session {
    pub access: String,
    pub refresh: String,
    pub expires: Instant,
    /// The user key, encrypted with the stretched master key.
    pub protected_key: Option<String>,
}

pub enum SignIn {
    Done(Session),
    /// The account has two-step login on: these providers (0 authenticator app, 1 email…).
    NeedsCode(Vec<u64>),
}

fn session(r: &Value) -> Result<Session> {
    Ok(Session {
        access: field(r, "access_token").as_str().ok_or("The server sent no session")?.to_owned(),
        refresh: field(r, "refresh_token").as_str().unwrap_or_default().to_owned(),
        expires: Instant::now() + Duration::from_secs(field(r, "expires_in").as_u64().unwrap_or(3600).saturating_sub(60)),
        protected_key: field(r, "key").as_str().map(str::to_owned),
    })
}

/// Signs in with the master password's hash; `code` is (provider, code) for two-step login.
pub fn sign_in(server: &Server, email: &str, hash: &str, device: &str, code: Option<(u64, &str)>) -> Result<SignIn> {
    let provider = code.map(|(p, _)| p.to_string()).unwrap_or_default();
    let mut form = vec![
        ("grant_type", "password"),
        ("username", email.trim()),
        ("password", hash),
        ("scope", "api offline_access"),
        ("client_id", CLIENT_ID),
        ("deviceType", DEVICE_TYPE),
        ("deviceIdentifier", device),
        ("deviceName", "Rune Passwords"),
    ];
    if let Some((_, c)) = code {
        form.extend([("twoFactorToken", c), ("twoFactorProvider", provider.as_str()), ("twoFactorRemember", "1")]);
    }
    let auth_email = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, email.trim());
    match request("POST", &server.identity("/connect/token")).set("Auth-Email", &auth_email).send_form(&form) {
        Ok(r) => Ok(SignIn::Done(session(&r.into_json().map_err(|e| e.to_string())?)?)),
        Err(ureq::Error::Status(400, r)) => {
            let body: Value = r.into_json().unwrap_or(Value::Null);
            let providers = field(&body, "twoFactorProviders");
            if let Some(list) = providers.as_array() {
                return Ok(SignIn::NeedsCode(
                    list.iter().filter_map(|p| p.as_u64().or_else(|| p.as_str().and_then(|s| s.parse().ok()))).collect(),
                ));
            }
            let said = field(&body, "error_description").as_str().or(field(field(&body, "errorModel"), "message").as_str());
            Err(match said {
                Some(s) if s.to_lowercase().contains("username or password") => "The email or master password isn't right".into(),
                Some(s) => s.to_owned(),
                None => "The server refused the sign-in".into(),
            })
        }
        Err(e) => Err(error(e)),
    }
}

/// A fresh session from the refresh token, when the last has run out.
pub fn renew(server: &Server, refresh: &str) -> Result<Session> {
    let form = [("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", refresh)];
    let r: Value = request("POST", &server.identity("/connect/token"))
        .send_form(&form)
        .map_err(error)?
        .into_json()
        .map_err(|e| e.to_string())?;
    let mut s = session(&r)?;
    if s.refresh.is_empty() {
        s.refresh = refresh.to_owned();
    }
    Ok(s)
}

/// The vault API with a session, renewed as it runs out.
pub struct Client {
    pub server: Server,
    pub session: Session,
}

impl Client {
    fn token(&mut self) -> Result<String> {
        if Instant::now() >= self.session.expires {
            self.session = renew(&self.server, &self.session.refresh)?;
        }
        Ok(format!("Bearer {}", self.session.access))
    }

    fn call(&mut self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = self.server.api(path);
        let run = |auth: &str| {
            let req = request(method, &url).set("Authorization", auth);
            match body {
                Some(b) => req.send_json(b),
                None => req.call(),
            }
            .map_err(Box::new)
        };
        let auth = self.token()?;
        let r = match run(&auth) {
            // Ended early (a password change elsewhere, a server restart): renew once.
            Err(e) if matches!(*e, ureq::Error::Status(401, _)) => {
                self.session = renew(&self.server, &self.session.refresh)?;
                run(&format!("Bearer {}", self.session.access))
            }
            r => r,
        }
        .map_err(|e| error(*e))?;
        let text = r.into_string().map_err(|e| e.to_string())?;
        Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).map_err(|e| e.to_string())? })
    }

    pub fn sync(&mut self) -> Result<Value> {
        self.call("GET", "/sync?excludeDomains=true", None)
    }

    /// Creates an item (`id` empty) or updates one; the server's copy comes back.
    pub fn save(&mut self, id: &str, cipher: &Value) -> Result<Value> {
        if id.is_empty() {
            self.call("POST", "/ciphers", Some(cipher))
        } else {
            self.call("PUT", &format!("/ciphers/{id}"), Some(cipher))
        }
    }

    /// To Recently Deleted, where it can be restored.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.call("PUT", &format!("/ciphers/{id}/delete"), None).map(drop)
    }

    pub fn restore(&mut self, id: &str) -> Result<()> {
        self.call("PUT", &format!("/ciphers/{id}/restore"), None).map(drop)
    }

    /// Gone for good.
    pub fn purge(&mut self, id: &str) -> Result<()> {
        self.call("DELETE", &format!("/ciphers/{id}"), None).map(drop)
    }

    #[cfg(test)]
    pub fn new_folder(&mut self, encrypted_name: &str) -> Result<Value> {
        self.call("POST", "/folders", Some(&json!({ "name": encrypted_name })))
    }
}

/// Creates an account, as a test's fixture on a local server.
#[cfg(test)]
pub fn register(server: &Server, email: &str, password: &str, kdf: Kdf) -> Result<()> {
    use crate::crypto::{master_key, new_account_keys, password_hash};
    let master = master_key(password, email, kdf)?;
    let (_, protected, public, private) = new_account_keys(&master);
    let Kdf::Pbkdf2 { iterations } = kdf else { return Err("tests register with PBKDF2".into()) };
    // With mail off, the server hands the verification token straight back.
    let token: Value = request("POST", &server.identity("/accounts/register/send-verification-email"))
        .set("Accept", "application/json")
        .send_json(json!({ "email": email, "name": "Test" }))
        .map_err(error)?
        .into_json()
        .map_err(|e| e.to_string())?;
    let body = json!({
        "email": email, "emailVerificationToken": token, "masterPasswordHash": password_hash(&master, password),
        "masterPasswordHint": null, "userSymmetricKey": protected, "kdf": 0, "kdfIterations": iterations,
        "userAsymmetricKeys": { "publicKey": public, "encryptedPrivateKey": private },
    });
    request("POST", &server.identity("/accounts/register/finish")).send_json(body).map_err(error).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_https_unless_local() {
        assert_eq!(Server::new("vault.example.com/").unwrap().0, "https://vault.example.com");
        assert!(Server::new("http://vault.example.com").is_err());
        assert!(Server::new("http://127.0.0.1:8222").is_ok());
        let cloud = Server::new("https://vault.bitwarden.com").unwrap();
        assert_eq!(cloud.api("/sync"), "https://api.bitwarden.com/sync");
        assert_eq!(Server::new("vault.example.com").unwrap().identity("/connect/token"), "https://vault.example.com/identity/connect/token");
    }
}

/// Against a real server: `RUNE_PASSWORDS_TEST_SERVER=http://127.0.0.1:8222 cargo test -- --ignored`,
/// with a throwaway Vaultwarden that allows sign-ups.
#[cfg(test)]
mod server_tests {
    use super::*;
    use crate::crypto::{SymKey, master_key, password_hash};
    use crate::vault::{Keys, Vault, encrypt_item};

    /// Registers `RUNE_PASSWORDS_UI_ACCOUNT` (email:password) on the test server, for driving
    /// the app itself against it.
    #[test]
    #[ignore]
    fn register_an_account_for_the_app() {
        let (Ok(base), Ok(account)) = (std::env::var("RUNE_PASSWORDS_TEST_SERVER"), std::env::var("RUNE_PASSWORDS_UI_ACCOUNT")) else {
            return;
        };
        let (email, password) = account.split_once(':').expect("email:password");
        register(&Server::new(&base).unwrap(), email, password, Kdf::Pbkdf2 { iterations: 600_000 }).unwrap();
    }

    #[test]
    #[ignore]
    fn signs_in_and_keeps_an_item_through_its_life() {
        let Ok(base) = std::env::var("RUNE_PASSWORDS_TEST_SERVER") else { return };
        let server = Server::new(&base).unwrap();
        let email = format!("test-{}@example.com", uuid::Uuid::new_v4().simple());
        let password = "correct horse battery staple";
        register(&server, &email, password, Kdf::Pbkdf2 { iterations: 100_000 }).unwrap();

        let (kdf, salt) = prelogin(&server, &email).unwrap();
        let master = master_key(password, &salt, kdf).unwrap();
        let wrong = master_key("nope", &salt, kdf).unwrap();
        let device = uuid::Uuid::new_v4().to_string();
        assert!(sign_in(&server, &email, &password_hash(&wrong, "nope"), &device, None).is_err());
        let SignIn::Done(session) = sign_in(&server, &email, &password_hash(&master, password), &device, None).unwrap() else {
            panic!("no two-step login on a new account")
        };
        let user = SymKey::stretch(&master).decrypt_key(session.protected_key.as_deref().unwrap()).unwrap();
        let mut client = Client { server: server.clone(), session };
        let keys = Keys::open(user, &client.sync().unwrap()).unwrap();

        let folder = client.new_folder(&keys.user.encrypt_str("Work")).unwrap();
        let mut item = crate::vault::Item::new_login(field(&folder, "id").as_str().map(str::to_owned));
        item.name = "Etsy".into();
        item.username = "lanchi@example.com".into();
        item.password = "first".into();
        item.uris = vec!["https://www.etsy.com".into()];
        client.save("", &encrypt_item(&item, &keys, "2026-10-31T00:00:00Z").unwrap()).unwrap();

        let vault = Vault::open(&client.sync().unwrap(), &keys);
        assert_eq!(vault.folders[0].name, "Work");
        let mut got = vault.items.iter().find(|i| i.name == "Etsy").cloned().expect("created");
        assert_eq!(got.password, "first");
        got.password = "second".into();
        client.save(&got.id.clone(), &encrypt_item(&got, &keys, "2026-10-31T00:00:01Z").unwrap()).unwrap();
        let sync = client.sync().unwrap();
        let vault = Vault::open(&sync, &keys);
        assert_eq!(vault.items[0].password, "second");
        let history = &field(&field(&sync, "ciphers")[0], "passwordHistory")[0];
        assert_eq!(keys.user.decrypt_str(field(history, "password").as_str().unwrap()).unwrap(), "first");

        client.delete(&got.id).unwrap();
        assert!(Vault::open(&client.sync().unwrap(), &keys).items[0].deleted.is_some(), "to Recently Deleted");
        client.restore(&got.id).unwrap();
        assert!(Vault::open(&client.sync().unwrap(), &keys).items[0].deleted.is_none());
        client.delete(&got.id).unwrap();
        client.purge(&got.id).unwrap();
        assert!(Vault::open(&client.sync().unwrap(), &keys).items.is_empty());

        // A new session from the refresh token, as after a restart.
        let renewed = renew(&server, &client.session.refresh).unwrap();
        assert!(!renewed.access.is_empty());
    }
}
