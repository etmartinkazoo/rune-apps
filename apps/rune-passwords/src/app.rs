//! The app's state, and what the views and keys do to it. The vault thread (`worker.rs`) holds
//! the keys; this holds what's shown: the decrypted items while the vault is open, and nothing
//! of them once it locks.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use rune::{TextEdit, Tx, Waker};

use crate::generator::generate;
use crate::settings::Settings;
use crate::strength::{Strength, reused, strength};
use crate::vault::{Item, Kind, Vault};
use crate::worker::{Cmd, Event};

const STATUS_SECS: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    SignIn,
    Unlock,
    Ready,
}

/// What the list shows: one of the sidebar's tiles, a folder or an organisation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Category {
    All,
    Passkeys,
    Codes,
    Notes,
    Security,
    Deleted,
    Folder(String),
    Org(String),
}

impl Category {
    pub fn title(&self, vault: Option<&Vault>) -> String {
        match self {
            Category::All => "All".into(),
            Category::Passkeys => "Passkeys".into(),
            Category::Codes => "Codes".into(),
            Category::Notes => "Notes".into(),
            Category::Security => "Security".into(),
            Category::Deleted => "Deleted".into(),
            Category::Folder(id) => vault.and_then(|v| v.folders.iter().find(|f| &f.id == id)).map_or("Folder".into(), |f| f.name.clone()),
            Category::Org(id) => vault.and_then(|v| v.orgs.iter().find(|o| &o.id == id)).map_or("Shared".into(), |o| o.name.clone()),
        }
    }
}

/// What Security says of a login's password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub strength: Strength,
    /// Other items with the same password.
    pub reused: usize,
}

impl Verdict {
    pub fn at_risk(&self) -> bool {
        self.strength == Strength::Weak || self.reused > 0
    }
}

/// An item being edited (or made): its fields as text fields.
pub struct Draft {
    pub item: Item,
    pub name: TextEdit,
    pub username: TextEdit,
    pub password: TextEdit,
    /// One address per line.
    pub websites: TextEdit,
    pub code: TextEdit,
    pub notes: TextEdit,
}

impl Draft {
    pub fn of(item: Item) -> Draft {
        Draft {
            name: TextEdit::new(&item.name),
            username: TextEdit::new(&item.username),
            password: TextEdit::new(&item.password),
            websites: TextEdit::new(item.uris.join("\n")),
            code: TextEdit::new(&item.totp),
            notes: TextEdit::new(&item.notes),
            item,
        }
    }

    /// The item as edited.
    pub fn item(&self) -> Item {
        let mut i = self.item.clone();
        i.name = self.name.text.trim().to_owned();
        i.username = self.username.text.trim().to_owned();
        i.password = self.password.text.clone();
        i.uris = self.websites.text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
        i.totp = self.code.text.trim().to_owned();
        i.notes = self.notes.text.clone();
        i
    }
}

/// "Click to copy" over a field's value: which field.
pub struct CopyTip {
    pub key: String,
}

/// A delete waiting for a yes.
pub struct Confirm {
    pub id: String,
    pub name: String,
    /// Delete Now: gone for good, not to Deleted.
    pub permanent: bool,
}

/// The fields of the sign-in, two-step and unlock screens.
pub struct Forms {
    pub server: TextEdit,
    pub email: TextEdit,
    pub password: TextEdit,
    pub code: TextEdit,
    /// Two-step login asked for a code from these providers.
    pub needs_code: Option<Vec<u64>>,
    pub unlock: TextEdit,
}

/// A kept account, as the account menu shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountEntry {
    pub id: String,
    pub email: String,
    pub server: String,
    /// The server's host, shown under the address.
    pub host: String,
}

impl AccountEntry {
    fn of(a: &crate::account::Account) -> AccountEntry {
        AccountEntry { id: a.id.clone(), email: a.email.clone(), server: a.server.0.clone(), host: a.host() }
    }
}

pub struct App {
    pub phase: Phase,
    pub settings: Settings,
    pub forms: Forms,
    /// Waiting for the vault thread (signing in, unlocking, saving).
    pub busy: bool,
    pub error: Option<String>,
    /// The account's address, for the unlock screen.
    pub email: String,
    /// Every account kept on this computer.
    pub accounts: Vec<AccountEntry>,
    /// The one in use.
    pub account_id: Option<String>,
    /// The sidebar's account menu is open.
    pub accounts_open: bool,
    /// Signing in to another account, with the one in use put aside (it can be gone back to).
    pub adding: bool,
    pub vault: Option<Vault>,
    pub verdicts: HashMap<String, Verdict>,
    pub category: Category,
    pub search: TextEdit,
    pub selected: Option<String>,
    pub draft: Option<Draft>,
    /// The selected item's password shows in the clear.
    pub revealed: bool,
    /// The last sync didn't reach the server.
    pub offline: bool,
    pub settings_open: bool,
    pub status: Option<(String, f64)>,
    pub clock: f64,
    /// A delete being asked about.
    pub confirm: Option<Confirm>,
    /// A list row slid open by a two-finger swipe.
    pub swipe: Option<crate::swipe::Swipe>,
    pub icons: crate::icons::Icons,
    pub copy_tip: Option<CopyTip>,
    /// Where the copyable values are drawn, by field, for placing `copy_tip`.
    pub field_rects: HashMap<String, [f32; 4]>,
    wake: Option<Waker>,
    cmds: Option<Sender<Cmd>>,
    events: Option<Receiver<Event>>,
}

pub fn init() -> App {
    let account = crate::account::active();
    let accounts: Vec<AccountEntry> = crate::account::list().iter().map(AccountEntry::of).collect();
    let forms = Forms {
        server: TextEdit::new(account.as_ref().map_or("", |a| a.server.0.as_str())),
        email: TextEdit::new(account.as_ref().map_or("", |a| a.email.as_str())),
        password: TextEdit::new("").secure(),
        code: TextEdit::new(""),
        needs_code: None,
        unlock: TextEdit::new("").secure(),
    };
    let mut s = App {
        phase: if account.is_some() { Phase::Unlock } else { Phase::SignIn },
        settings: Settings::load(),
        forms,
        busy: false,
        error: None,
        account_id: account.as_ref().map(|a| a.id.clone()),
        email: account.map(|a| a.email).unwrap_or_default(),
        accounts,
        accounts_open: false,
        adding: false,
        vault: None,
        verdicts: HashMap::new(),
        category: Category::All,
        search: TextEdit::new(""),
        selected: None,
        draft: None,
        revealed: false,
        offline: false,
        settings_open: false,
        status: None,
        clock: 0.0,
        confirm: None,
        swipe: None,
        icons: crate::icons::Icons::default(),
        copy_tip: None,
        field_rects: HashMap::new(),
        wake: None,
        cmds: None,
        events: None,
    };
    match s.phase {
        Phase::SignIn => s.forms.server.focused = true,
        _ => s.forms.unlock.focused = true,
    }
    s
}

pub fn setup(s: &mut App, wake: Waker) {
    s.wake = Some(wake.clone());
    let (cmds, events) = crate::worker::spawn(wake);
    s.cmds = Some(cmds);
    s.events = Some(events);
    // A demo scene may have opened its vault already.
    if let Some(v) = s.vault.take() {
        s.want_icons(&v);
        s.vault = Some(v);
    }
}

pub fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = false;
    let events: Vec<Event> = s.events.as_ref().map(|e| e.try_iter().collect()).unwrap_or_default();
    for e in events {
        s.apply(e, tx);
        changed = true;
    }
    changed |= s.icons.poll();
    changed |= s.settle_quiet_swipe(tx);
    if s.status.as_ref().is_some_and(|(_, until)| clock >= *until) {
        s.status = None;
        changed = true;
    }
    // Locks once nothing has been typed or clicked for the set time.
    if s.phase == Phase::Ready && s.settings.lock_minutes > 0 {
        let idle = Duration::from_secs(u64::from(s.settings.lock_minutes) * 60);
        match rune::last_input() {
            Some(at) if at.elapsed() >= idle => {
                s.lock();
                changed = true;
            }
            _ => tx.wake_after(idle.min(Duration::from_secs(30))),
        }
    }
    // A verification code on screen counts down.
    if s.phase == Phase::Ready && s.selected_item().is_some_and(|i| !i.totp.is_empty()) {
        tx.wake_after(Duration::from_secs(1));
        changed = true;
    }
    changed
}

impl App {
    fn send(&self, cmd: Cmd) {
        if let Some(c) = &self.cmds {
            let _ = c.send(cmd);
        }
    }

    pub fn say(&mut self, text: impl Into<String>, tx: &mut Tx) {
        self.status = Some((text.into(), self.clock + STATUS_SECS));
        tx.wake_after(Duration::from_secs_f64(STATUS_SECS));
    }

    fn apply(&mut self, e: Event, tx: &mut Tx) {
        match e {
            Event::NeedsCode(providers) => {
                self.busy = false;
                self.forms.needs_code = Some(providers);
                self.forms.code.focused = true;
            }
            Event::WrongPassword => {
                self.busy = false;
                self.error = Some("That isn't the master password".into());
                self.forms.unlock.focused = true;
            }
            // Only while opening or open: not one that lands after locking or switching.
            Event::Vault(v) if self.phase == Phase::Ready || self.busy => self.open(*v),
            Event::Vault(_) => {}
            Event::Saved(id) => {
                self.busy = false;
                self.draft = None;
                self.selected = Some(id);
                self.say("Saved", tx);
            }
            Event::Offline(e) => {
                self.offline = true;
                self.say(format!("Offline: showing the vault as last synced ({e})"), tx);
            }
            Event::Error(e) => {
                self.busy = false;
                if self.phase == Phase::Ready {
                    self.say(e, tx);
                } else {
                    self.error = Some(e);
                }
            }
            Event::Locked => {}
            Event::SignedOut => {
                self.refresh_accounts();
                self.adding = false;
                self.phase = Phase::SignIn;
                self.forms.password = TextEdit::new("").secure();
                self.forms.server.focused = true;
                // The next account may be on another server.
                self.icons = crate::icons::Icons::default();
            }
            Event::Switched(id) => {
                self.refresh_accounts();
                self.locked_on(&id);
            }
        }
    }

    fn open(&mut self, v: Vault) {
        self.busy = false;
        self.error = None;
        self.offline = false;
        if self.phase != Phase::Ready {
            self.phase = Phase::Ready;
            self.adding = false;
            self.refresh_accounts();
            self.forms.password = TextEdit::new("").secure();
            self.forms.unlock = TextEdit::new("").secure();
            self.forms.needs_code = None;
            self.forms.code = TextEdit::new("");
        }
        self.email = v.email.clone();
        self.verdicts = verdicts(&v);
        self.want_icons(&v);
        if self.selected.as_ref().is_some_and(|id| !v.items.iter().any(|i| &i.id == id)) {
            self.selected = None;
        }
        self.vault = Some(v);
        if self.selected.is_none() {
            self.selected = self.items().first().map(|i| i.id.clone());
        }
    }

    /// Asks the server for the icon of each site in `v`, starting the fetching on the first vault.
    fn want_icons(&mut self, v: &Vault) {
        if !self.icons.started()
            && let Some(wake) = self.wake.clone()
        {
            // No account (the demo scenes): icons kept on disk only.
            let server = self.current_account().map(|a| a.server.clone()).unwrap_or_default();
            self.icons.start(server, wake);
        }
        let hosts: Vec<String> = v.items.iter().filter(|i| i.deleted.is_none()).filter_map(Item::host).collect();
        self.icons.want(hosts.iter().map(String::as_str));
    }

    // Signing in and out, locking

    pub fn sign_in(&mut self) {
        let f = &self.forms;
        if f.server.text.trim().is_empty() || f.email.text.trim().is_empty() || f.password.text.is_empty() {
            self.error = Some("Fill in the server, email and master password".into());
            return;
        }
        let code = f.needs_code.as_ref().map(|p| (p.first().copied().unwrap_or(0), f.code.text.trim().to_owned()));
        self.busy = true;
        self.error = None;
        self.send(Cmd::SignIn { server: f.server.text.clone(), email: f.email.text.clone(), password: f.password.text.clone(), code });
    }

    pub fn unlock(&mut self) {
        if self.forms.unlock.text.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        self.send(Cmd::Unlock { password: self.forms.unlock.text.clone() });
        self.forms.unlock = TextEdit::new("").secure();
    }

    /// Closes the vault: the decrypted items go from here, and the keys from the vault thread.
    pub fn lock(&mut self) {
        if self.phase != Phase::Ready {
            return;
        }
        self.send(Cmd::Lock);
        self.forget_vault();
        self.phase = Phase::Unlock;
        self.forms.unlock = TextEdit::new("").secure();
        self.forms.unlock.focused = true;
    }

    /// What's shown of an open vault goes.
    fn forget_vault(&mut self) {
        self.vault = None;
        self.verdicts.clear();
        self.draft = None;
        self.revealed = false;
        self.settings_open = false;
        self.accounts_open = false;
        self.confirm = None;
        self.swipe = None;
        self.copy_tip = None;
        self.offline = false;
    }

    pub fn sign_out(&mut self) {
        self.lock();
        self.accounts_open = false;
        self.send(Cmd::SignOut);
    }

    // Accounts

    pub fn current_account(&self) -> Option<&AccountEntry> {
        let id = self.account_id.as_ref()?;
        self.accounts.iter().find(|a| &a.id == id)
    }

    fn refresh_accounts(&mut self) {
        self.accounts = crate::account::list().iter().map(AccountEntry::of).collect();
        self.account_id = crate::account::active().map(|a| a.id);
    }

    /// The unlock screen for kept account `id`.
    fn locked_on(&mut self, id: &str) {
        self.forget_vault();
        self.account_id = Some(id.to_owned());
        self.email = self.current_account().map(|a| a.email.clone()).unwrap_or_default();
        self.adding = false;
        self.busy = false;
        self.error = None;
        self.phase = Phase::Unlock;
        self.forms.unlock = TextEdit::new("").secure();
        self.forms.unlock.focused = true;
        self.forms.password = TextEdit::new("").secure();
        self.forms.needs_code = None;
        self.forms.code = TextEdit::new("");
        // Its server may be another, with icons of its own.
        self.icons = crate::icons::Icons::default();
    }

    /// Locks, and moves to kept account `id`, whose master password then opens it.
    pub fn switch_account(&mut self, id: &str) {
        self.accounts_open = false;
        if self.busy || (self.account_id.as_deref() == Some(id) && !self.adding) || !self.accounts.iter().any(|a| a.id == id) {
            return;
        }
        self.send(Cmd::Switch(id.to_owned()));
        self.locked_on(id);
    }

    /// Locks, and signs in to another account; the one in use stays kept.
    pub fn add_account(&mut self) {
        self.accounts_open = false;
        if self.busy {
            return;
        }
        self.send(Cmd::AddAccount);
        let server = self.current_account().map(|a| a.server.clone()).unwrap_or_default();
        self.forget_vault();
        self.adding = true;
        self.error = None;
        self.phase = Phase::SignIn;
        self.forms.email = TextEdit::new("");
        self.forms.password = TextEdit::new("").secure();
        self.forms.needs_code = None;
        self.forms.code = TextEdit::new("");
        // Most often another account on the same server.
        self.forms.server = TextEdit::new(&server);
        if server.is_empty() {
            self.forms.server.focused = true;
        } else {
            self.forms.email.focused = true;
        }
        self.icons = crate::icons::Icons::default();
    }

    /// Back from adding an account to the one put aside.
    pub fn cancel_add(&mut self) {
        if let Some(id) = self.account_id.clone().filter(|_| self.adding && !self.busy) {
            self.switch_account(&id);
        }
    }

    pub fn sync(&mut self) {
        self.send(Cmd::Sync);
    }

    // What the list shows

    /// The items of the category, matching the search, by name.
    pub fn items(&self) -> Vec<&Item> {
        let Some(v) = &self.vault else { return Vec::new() };
        let q = self.search.text.trim().to_lowercase();
        v.items
            .iter()
            .filter(|i| self.in_category(i, &self.category))
            .filter(|i| {
                q.is_empty()
                    || i.name.to_lowercase().contains(&q)
                    || i.username.to_lowercase().contains(&q)
                    || i.uris.iter().any(|u| u.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn in_category(&self, i: &Item, c: &Category) -> bool {
        if *c == Category::Deleted {
            return i.deleted.is_some();
        }
        if i.deleted.is_some() {
            return false;
        }
        match c {
            Category::All => true,
            Category::Passkeys => !i.passkeys.is_empty(),
            Category::Codes => !i.totp.is_empty(),
            Category::Notes => i.kind == Kind::Note,
            Category::Security => self.verdicts.get(&i.id).is_some_and(Verdict::at_risk),
            Category::Deleted => unreachable!("handled above"),
            Category::Folder(id) => i.folder.as_ref() == Some(id),
            Category::Org(id) => i.org.as_ref() == Some(id),
        }
    }

    pub fn count(&self, c: &Category) -> usize {
        self.vault.as_ref().map_or(0, |v| v.items.iter().filter(|i| self.in_category(i, c)).count())
    }

    pub fn selected_item(&self) -> Option<&Item> {
        let id = self.selected.as_ref()?;
        self.vault.as_ref()?.items.iter().find(|i| &i.id == id)
    }

    pub fn choose(&mut self, c: Category) {
        self.category = c;
        self.draft = None;
        self.revealed = false;
        let items = self.items();
        if !self.selected.as_ref().is_some_and(|s| items.iter().any(|i| &i.id == s)) {
            self.selected = items.first().map(|i| i.id.clone());
        }
    }

    pub fn select(&mut self, id: &str) {
        self.copy_tip = None;
        if self.close_swipe() {
            return; // a click on a swiped-open list first closes it
        }
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_owned());
            self.draft = None;
            self.revealed = false;
        }
    }

    /// The row `by` places from the selected one, in the list as shown.
    pub fn step(&mut self, by: isize) {
        let ids: Vec<String> = self.items().iter().map(|i| i.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| ids.iter().position(|i| i == s));
        let next = match at {
            Some(i) => (i as isize + by).clamp(0, ids.len() as isize - 1) as usize,
            None => 0,
        };
        let id = ids[next].clone();
        self.select(&id);
    }

    // Copying

    pub fn copy(&mut self, what: &str, value: String, tx: &mut Tx) {
        if value.is_empty() {
            return;
        }
        tx.copy(value);
        self.say(format!("Copied {what}"), tx);
    }

    // Editing

    pub fn edit(&mut self) {
        if let Some(i) = self.selected_item().filter(|i| can_edit(i)).cloned() {
            let mut d = Draft::of(i);
            d.name.focused = true;
            self.draft = Some(d);
        }
    }

    pub fn new_item(&mut self) {
        let folder = match &self.category {
            Category::Folder(id) => Some(id.clone()),
            _ => None,
        };
        let mut d = Draft::of(Item::new_login(folder));
        d.password = TextEdit::new(generate());
        d.name.focused = true;
        self.draft = Some(d);
        self.revealed = true;
    }

    pub fn cancel_edit(&mut self) -> bool {
        self.draft.take().is_some()
    }

    pub fn generate_password(&mut self) {
        if let Some(d) = &mut self.draft {
            d.password = TextEdit::new(generate());
            self.revealed = true;
        }
    }

    pub fn save(&mut self, tx: &mut Tx) {
        let Some(d) = &self.draft else { return };
        let item = d.item();
        if item.name.is_empty() {
            let name = item.host().unwrap_or_default();
            if name.is_empty() {
                self.say("Give it a name or a website", tx);
                return;
            }
            if let Some(d) = &mut self.draft {
                d.name = TextEdit::new(&name);
            }
            return self.save(tx);
        }
        self.busy = true;
        self.send(Cmd::Save(Box::new(item)));
    }

    /// Asks before deleting item `id`: to Deleted, or for good if it's there already.
    pub fn ask_delete(&mut self, id: &str) {
        let Some(item) = self.vault.as_ref().and_then(|v| v.items.iter().find(|i| i.id == id)).filter(|i| can_delete(i)) else { return };
        self.swipe = None;
        self.confirm = Some(Confirm { id: id.to_owned(), name: item.name.clone(), permanent: item.deleted.is_some() });
    }

    /// The selected item, from the keyboard.
    pub fn ask_delete_selected(&mut self) {
        if let Some(id) = self.selected.clone() {
            self.ask_delete(&id);
        }
    }

    /// The yes to `confirm`.
    pub fn confirm_delete(&mut self) {
        let Some(c) = self.confirm.take() else { return };
        if self.draft.as_ref().is_some_and(|d| d.item.id == c.id) {
            self.draft = None;
        }
        if c.permanent {
            if self.selected.as_ref() == Some(&c.id) {
                self.selected = None;
            }
            self.send(Cmd::Purge(c.id));
        } else {
            self.send(Cmd::Delete(c.id));
        }
    }

    pub fn restore_selected(&mut self) {
        if let Some(id) = self.selected.clone() {
            self.restore_item(&id);
        }
    }

    pub fn restore_item(&mut self, id: &str) {
        self.send(Cmd::Restore(id.to_owned()));
    }

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings), tx: &mut Tx) {
        f(&mut self.settings);
        if let Err(e) = self.settings.save() {
            self.say(format!("Couldn't save settings: {e}"), tx);
        }
    }

    /// Opens `v` as if unlocked: demo scenes and tests.
    pub fn show_vault(&mut self, v: Vault) {
        self.open(v);
    }
}

/// Logins are edited here; notes, cards and identities show their details and are edited in
/// the Bitwarden apps for now.
pub fn can_edit(i: &Item) -> bool {
    i.editable && i.deleted.is_none() && i.kind == Kind::Login
}

/// Items the account may delete (shared ones can be read-only).
pub fn can_delete(i: &Item) -> bool {
    i.editable
}

/// Security's verdict on each login with a password.
fn verdicts(v: &Vault) -> HashMap<String, Verdict> {
    let live: Vec<&Item> = v.items.iter().filter(|i| i.deleted.is_none() && i.kind == Kind::Login && !i.password.is_empty()).collect();
    let counts = reused(live.iter().map(|i| i.password.as_str()));
    let mut seen = HashSet::new();
    live.iter()
        .filter(|i| seen.insert(i.id.clone()))
        .map(|i| {
            let host = i.host().unwrap_or_default();
            let verdict = Verdict {
                strength: strength(&i.password, &[&i.name, &i.username, &host]),
                reused: counts.get(i.password.as_str()).map_or(0, |n| n - 1),
            };
            (i.id.clone(), verdict)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        crate::account::use_scratch_dirs();
        let mut s = init();
        s.show_vault(crate::demo::vault());
        s
    }

    #[test]
    fn categories_count_what_they_hold() {
        let s = app();
        assert_eq!(s.phase, Phase::Ready);
        assert_eq!(s.count(&Category::Deleted), 1);
        assert_eq!(s.count(&Category::All), s.vault.as_ref().unwrap().items.len() - 1, "deleted items aren't in All");
        assert_eq!(s.count(&Category::Codes), 2);
        assert_eq!(s.count(&Category::Passkeys), 2);
        assert_eq!(s.count(&Category::Folder("work".into())), 2);
        // Kobu and LinkedIn share a password; Target's is "password1".
        let at_risk: Vec<&str> = s.vault.as_ref().unwrap().items.iter().filter(|i| s.in_category(i, &Category::Security)).map(|i| i.name.as_str()).collect();
        assert_eq!(at_risk, ["Kobu", "LinkedIn", "Target"]);
        assert_eq!(s.verdicts["linkedin"].reused, 1);
    }

    #[test]
    fn search_and_steps_follow_the_list_shown() {
        let mut s = app();
        s.search = TextEdit::new("linked");
        assert_eq!(s.items().iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["LinkedIn"]);
        s.search = TextEdit::new("");
        s.choose(Category::Codes);
        assert_eq!(s.selected.as_deref(), Some("etsy"), "the first of the category");
        s.step(1);
        assert_eq!(s.selected.as_deref(), Some("quickbooks"));
        s.step(5);
        assert_eq!(s.selected.as_deref(), Some("quickbooks"), "stays at the end");
    }

    #[test]
    fn a_draft_reads_back_its_fields_and_locking_forgets_the_vault() {
        let mut s = app();
        s.select("etsy");
        s.edit();
        let d = s.draft.as_mut().unwrap();
        d.websites = TextEdit::new(" etsy.com \n\nshop.etsy.com");
        d.username = TextEdit::new("  me@example.com ");
        let item = d.item();
        assert_eq!(item.uris, ["etsy.com", "shop.etsy.com"]);
        assert_eq!(item.username, "me@example.com");
        s.ask_delete("etsy");
        s.lock();
        assert!(s.vault.is_none() && s.draft.is_none() && s.verdicts.is_empty() && s.confirm.is_none());
        assert_eq!(s.phase, Phase::Unlock);
    }

    #[test]
    fn deleting_asks_first() {
        let mut s = app();
        s.select("etsy");
        s.edit();
        s.ask_delete("etsy");
        let c = s.confirm.as_ref().unwrap();
        assert_eq!((c.name.as_str(), c.permanent), ("Etsy", false));
        assert!(s.draft.is_some(), "nothing happens until the yes");
        s.confirm_delete();
        assert!(s.confirm.is_none() && s.draft.is_none());
        s.ask_delete("old");
        assert!(s.confirm.as_ref().unwrap().permanent, "a deleted item goes for good");
        s.ask_delete("nothing");
        assert_eq!(s.confirm.as_ref().unwrap().id, "old", "an unknown id asks nothing");
    }
}
