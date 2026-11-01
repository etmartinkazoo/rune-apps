//! The thread that holds the vault's keys and talks to the server, so neither key derivation
//! (deliberately slow) nor the network ever holds up the window. The UI gets decrypted items;
//! the keys stay here, and go when the vault locks.

use std::sync::mpsc::{Receiver, Sender, channel};

use rune::Waker;
use serde_json::Value;
use zeroize::Zeroize;

use crate::account::{self, Account};
use crate::api::{self, Client, Server, SignIn};
use crate::crypto::{Result, SymKey, master_key, password_hash};
use crate::vault::{Item, Keys, Vault, encrypt_item, field};

pub enum Cmd {
    SignIn { server: String, email: String, password: String, code: Option<(u64, String)> },
    Unlock { password: String },
    Sync,
    /// Creates (an item without an id) or updates an item.
    Save(Box<Item>),
    Delete(String),
    Restore(String),
    Purge(String),
    Lock,
    /// Signs the account in use out, moving to another kept one if there is one.
    SignOut,
    /// Locks and moves to the kept account with this id.
    Switch(String),
    /// Locks and puts the account in use aside, for signing in to another.
    AddAccount,
}

pub enum Event {
    /// Two-step login is on: the code from one of these providers.
    NeedsCode(Vec<u64>),
    /// The master password didn't open the vault.
    WrongPassword,
    /// Open: from the cache at once, then again as each sync lands.
    Vault(Box<Vault>),
    /// An item was saved under this id.
    Saved(String),
    /// The server can't be reached: the vault is as last synced.
    Offline(String),
    Error(String),
    Locked,
    /// No account is left.
    SignedOut,
    /// Now on the kept account with this id, locked (after signing out of another).
    Switched(String),
}

pub fn spawn(wake: Waker) -> (Sender<Cmd>, Receiver<Event>) {
    let (cmds, rx) = channel();
    let (tx, events) = channel();
    std::thread::Builder::new()
        .name("vault".into())
        .spawn(move || Worker { events: tx, wake, account: account::active(), keys: None, client: None, sync: None }.run(rx))
        .expect("spawn the vault thread");
    (cmds, events)
}

struct Worker {
    events: Sender<Event>,
    wake: Waker,
    account: Option<Account>,
    keys: Option<Keys>,
    client: Option<Client>,
    /// The last sync, as the server sent it (ciphertext).
    sync: Option<Value>,
}

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        for cmd in rx {
            let result = match cmd {
                Cmd::SignIn { server, email, mut password, code } => {
                    let r = self.sign_in(&server, &email, &password, code);
                    password.zeroize();
                    r
                }
                Cmd::Unlock { mut password } => {
                    let r = self.unlock(&password);
                    password.zeroize();
                    r
                }
                Cmd::Sync => self.sync(),
                Cmd::Save(item) => self.save(&item),
                Cmd::Delete(id) => self.with_client(|c| c.delete(&id)).and_then(|()| self.sync()),
                Cmd::Restore(id) => self.with_client(|c| c.restore(&id)).and_then(|()| self.sync()),
                Cmd::Purge(id) => self.with_client(|c| c.purge(&id)).and_then(|()| self.sync()),
                Cmd::Lock => {
                    self.lock();
                    Ok(())
                }
                Cmd::SignOut => {
                    self.lock();
                    self.client = None;
                    if let Some(a) = self.account.take() {
                        account::forget(&a.id);
                    }
                    match account::list().into_iter().next() {
                        Some(next) => {
                            let _ = account::set_active(&next.id);
                            let id = next.id.clone();
                            self.account = Some(next);
                            self.send(Event::Switched(id));
                        }
                        None => self.send(Event::SignedOut),
                    }
                    Ok(())
                }
                Cmd::Switch(id) => {
                    self.lock();
                    self.client = None;
                    match account::load(&id) {
                        Some(a) => {
                            let _ = account::set_active(&a.id);
                            self.account = Some(a);
                            Ok(())
                        }
                        None => Err("That account isn't kept on this computer any more".to_owned()),
                    }
                }
                Cmd::AddAccount => {
                    self.lock();
                    self.client = None;
                    self.account = None;
                    Ok(())
                }
            };
            if let Err(e) = result {
                self.send(Event::Error(e));
            }
        }
    }

    fn lock(&mut self) {
        self.keys = None;
        self.sync = None;
        self.send(Event::Locked);
    }

    fn sign_in(&mut self, server: &str, email: &str, password: &str, code: Option<(u64, String)>) -> Result<()> {
        let server = Server::new(server)?;
        let email = email.trim().to_lowercase();
        let (kdf, salt) = api::prelogin(&server, &email)?;
        let master = master_key(password, &salt, kdf)?;
        // Signing in again to a kept account keeps its id and device.
        let known = account::list().into_iter().find(|a| a.server == server && a.email == email);
        let id = known.as_ref().map_or_else(account::new_id, |a| a.id.clone());
        let device = known.as_ref().map_or_else(|| uuid::Uuid::new_v4().to_string(), |a| a.device.clone());
        let code = code.as_ref().map(|(p, c)| (*p, c.trim()));
        let session = match api::sign_in(&server, &email, &password_hash(&master, password), &device, code)? {
            SignIn::Done(s) => s,
            SignIn::NeedsCode(providers) => {
                self.send(Event::NeedsCode(providers));
                return Ok(());
            }
        };
        let protected = session.protected_key.clone().ok_or("The server sent no vault key")?;
        let user = SymKey::stretch(&master).decrypt_key(&protected)?;
        let a = Account { id, server: server.clone(), email, kdf, salt, device, protected_key: protected, refresh: Some(session.refresh.clone()) };
        account::save(&a).map_err(|e| format!("Couldn't save the account: {e}"))?;
        let _ = account::set_active(&a.id);
        self.account = Some(a);
        self.client = Some(Client { server, session });
        let sync = self.with_client(|c| c.sync())?;
        self.keys = Some(Keys::open(user, &sync)?);
        self.publish(sync);
        Ok(())
    }

    /// Opens the vault with the master password: from the last sync at once, then synced.
    fn unlock(&mut self, password: &str) -> Result<()> {
        let a = self.account.clone().ok_or("No account to unlock")?;
        let master = master_key(password, &a.salt, a.kdf)?;
        let Ok(user) = SymKey::stretch(&master).decrypt_key(&a.protected_key) else {
            self.send(Event::WrongPassword);
            return Ok(());
        };
        let cached = account::cached_sync(&a.id).unwrap_or(Value::Null);
        self.keys = Some(Keys::open(user.clone(), &cached)?);
        if !cached.is_null() {
            self.sync = Some(cached.clone());
            self.send(Event::Vault(Box::new(Vault::open(&cached, self.keys.as_ref().expect("set above")))));
        }
        match self.with_client(|c| c.sync()) {
            Ok(sync) => {
                // Organisation keys may have come or gone since the cache.
                self.keys = Some(Keys::open(user, &sync)?);
                self.publish(sync);
            }
            Err(e) => self.send(Event::Offline(e)),
        }
        Ok(())
    }

    fn sync(&mut self) -> Result<()> {
        if self.keys.is_none() {
            return Ok(());
        }
        let sync = self.with_client(|c| c.sync())?;
        self.publish(sync);
        Ok(())
    }

    /// Keeps `sync`, and sends the vault it opens to.
    fn publish(&mut self, sync: Value) {
        if let Some(a) = &self.account {
            account::cache_sync(&a.id, &sync);
        }
        if let Some(keys) = &self.keys {
            self.send(Event::Vault(Box::new(Vault::open(&sync, keys))));
        }
        self.sync = Some(sync);
    }

    fn save(&mut self, item: &Item) -> Result<()> {
        let keys = self.keys.as_ref().ok_or("The vault is locked")?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let cipher = encrypt_item(item, keys, &now)?;
        let id = item.id.clone();
        let saved = self.with_client(|c| c.save(&id, &cipher))?;
        let new_id = field(&saved, "id").as_str().unwrap_or(&id).to_owned();
        // The server's copy goes in the kept sync in place of the old, without a full sync.
        if let Some(mut sync) = self.sync.take() {
            if let Some(list) = sync.get_mut("ciphers").and_then(Value::as_array_mut) {
                list.retain(|c| field(c, "id").as_str() != Some(&new_id));
                list.push(saved);
            }
            self.publish(sync);
        }
        self.send(Event::Saved(new_id));
        Ok(())
    }

    /// Runs `f` with a client, signing the session in again from the refresh token if there's
    /// none (after a restart).
    fn with_client<T>(&mut self, f: impl FnOnce(&mut Client) -> Result<T>) -> Result<T> {
        if self.client.is_none() {
            let a = self.account.as_mut().ok_or("Not signed in")?;
            let token = a.refresh.clone().ok_or("Sign in again to sync")?;
            let session = api::renew(&a.server, &token)?;
            if session.refresh != token {
                a.refresh = Some(session.refresh.clone());
                let _ = account::save(a);
            }
            self.client = Some(Client { server: a.server.clone(), session });
        }
        f(self.client.as_mut().expect("set above"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Through the worker against a throwaway Vaultwarden (see `api::server_tests`): sign in,
    /// save, lock, unlock offline-first, a wrong password.
    #[test]
    #[ignore]
    fn signs_in_saves_locks_and_unlocks() {
        let Ok(base) = std::env::var("RUNE_PASSWORDS_TEST_SERVER") else { return };
        account::use_scratch_dirs();
        let email = format!("w-{}@example.com", uuid::Uuid::new_v4().simple());
        api::register(&Server::new(&base).unwrap(), &email, "pw-1", crate::crypto::Kdf::Pbkdf2 { iterations: 100_000 }).unwrap();
        let (cmds, events) = spawn(std::sync::Arc::new(|| {}));
        let next = || events.recv_timeout(std::time::Duration::from_secs(30)).expect("an answer");
        let vault = |e: Event| match e {
            Event::Vault(v) => v,
            Event::Error(e) => panic!("{e}"),
            _ => panic!("expected the vault"),
        };
        let email_kept = email.clone();
        cmds.send(Cmd::SignIn { server: base, email, password: "pw-1".into(), code: None }).unwrap();
        assert!(vault(next()).items.is_empty());
        let mut item = Item::new_login(None);
        item.name = "Etsy".into();
        item.password = "p".into();
        cmds.send(Cmd::Save(Box::new(item))).unwrap();
        assert_eq!(vault(next()).items[0].name, "Etsy");
        assert!(matches!(next(), Event::Saved(id) if !id.is_empty()));
        cmds.send(Cmd::Lock).unwrap();
        assert!(matches!(next(), Event::Locked));
        cmds.send(Cmd::Unlock { password: "wrong".into() }).unwrap();
        assert!(matches!(next(), Event::WrongPassword));
        cmds.send(Cmd::Unlock { password: "pw-1".into() }).unwrap();
        assert_eq!(vault(next()).items[0].name, "Etsy", "from the cache");
        assert_eq!(vault(next()).items[0].name, "Etsy", "then synced");
        cmds.send(Cmd::SignOut).unwrap();
        assert!(matches!(next(), Event::Locked));
        assert!(matches!(next(), Event::SignedOut | Event::Switched(_)));
        assert!(account::list().iter().all(|a| a.email != email_kept));
    }
}
