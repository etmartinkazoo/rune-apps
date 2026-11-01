//! Sites' icons for the list and the cards, as the Bitwarden apps get them: from the server's
//! icon service (`/icons/<host>/icon.png`), which fetches the site's own icon. So it's the
//! vault's server that learns the sites, as it holds them anyway, not a third party. Kept on
//! disk for a month; a site without one keeps its letter tile.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rune::{Image, ImageCache, Waker};

/// Requests at once: icons are small, and the server fetches each from its site.
const THREADS: usize = 4;
/// A kept icon is fetched again after this long.
const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);

#[derive(Default)]
pub struct Icons {
    cache: Option<ImageCache>,
    /// Decoded icons by host; None for a host with none.
    pub images: HashMap<String, Option<Image>>,
    asked: HashSet<String>,
    jobs: Option<Sender<String>>,
    done: Option<Receiver<(String, Option<Vec<u8>>)>>,
    /// Icons still decoding: the view runs again when they're ready.
    decoding: Vec<String>,
}

fn dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("rune-passwords").join("icons")
}

/// A host as a file name: letters, digits, dots and dashes only.
fn file_of(host: &str) -> Option<PathBuf> {
    let ok = !host.is_empty() && host.len() < 254 && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') && !host.starts_with('.');
    ok.then(|| dir().join(format!("{host}.png")))
}

/// The server's answer: an empty one when it has no icon, None when it couldn't be reached.
fn get(server: &str, host: &str) -> Option<Vec<u8>> {
    let url = format!("{}/icons/{host}/icon.png", server.trim_end_matches('/'));
    let r = match ureq::get(&url).timeout(Duration::from_secs(15)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(..)) => return Some(Vec::new()),
        Err(_) => return None,
    };
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(r.into_reader(), 512 * 1024), &mut bytes).ok()?;
    Some(bytes)
}

/// What the server answers for a site with no icon (Vaultwarden sends a placeholder rather
/// than a 404): asked once, of a site that can't exist.
type Placeholder = Arc<OnceLock<Option<Vec<u8>>>>;

fn fetch(server: &str, host: &str, placeholder: &Placeholder) -> Option<Vec<u8>> {
    let file = file_of(host)?;
    if let Ok(meta) = std::fs::metadata(&file)
        && meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < KEEP)
    {
        // An empty file says the site has no icon.
        return std::fs::read(&file).ok().filter(|b| !b.is_empty());
    }
    if server.is_empty() {
        return None;
    }
    let none = placeholder.get_or_init(|| get(server, "no-such-site.invalid"));
    // Unreachable: asked again next time rather than kept as having none.
    let got = get(server, host)?;
    let got = (got.len() > 64 && Some(&got) != none.as_ref()).then_some(got);
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(&file, got.as_deref().unwrap_or_default());
    got
}

impl Icons {
    /// Starts the fetching threads for `server`.
    pub fn started(&self) -> bool {
        self.jobs.is_some()
    }

    pub fn start(&mut self, server: String, wake: Waker) {
        self.cache = Some(ImageCache::new(wake.clone()));
        let (jobs, rx) = channel::<String>();
        let (tx, done) = channel();
        let rx = Arc::new(Mutex::new(rx));
        let placeholder = Placeholder::default();
        for _ in 0..THREADS {
            let (rx, tx, server, wake, placeholder) = (rx.clone(), tx.clone(), server.clone(), wake.clone(), placeholder.clone());
            std::thread::Builder::new()
                .name("icons".into())
                .spawn(move || {
                    loop {
                        let job = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok(host) = job else { return };
                        let bytes = fetch(&server, &host, &placeholder);
                        if tx.send((host, bytes)).is_err() {
                            return;
                        }
                        wake();
                    }
                })
                .expect("spawn an icon thread");
        }
        self.jobs = Some(jobs);
        self.done = Some(done);
    }

    /// Asks for each host's icon once.
    pub fn want<'a>(&mut self, hosts: impl IntoIterator<Item = &'a str>) {
        let Some(jobs) = &self.jobs else { return };
        for h in hosts {
            if file_of(h).is_some() && self.asked.insert(h.to_owned()) {
                let _ = jobs.send(h.to_owned());
            }
        }
    }

    /// Takes in what was fetched and decoded. True when the view should run again.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let got: Vec<(String, Option<Vec<u8>>)> = self.done.as_ref().map(|d| d.try_iter().collect()).unwrap_or_default();
        for (host, bytes) in got {
            let image = match (bytes, &self.cache) {
                (Some(b), Some(cache)) => {
                    self.decoding.push(host.clone());
                    Some(cache.load(b))
                }
                _ => None,
            };
            self.images.insert(host, image);
            changed = true;
        }
        let images = &self.images;
        let before = self.decoding.len();
        self.decoding.retain(|h| images.get(h).and_then(Option::as_ref).is_some_and(|i| !i.is_ready() && i.error().is_none()));
        changed | (self.decoding.len() != before)
    }

    /// The decoded icon for `host`, if there's one.
    pub fn get(&self, host: &str) -> Option<&Image> {
        self.images.get(host)?.as_ref().filter(|i| i.is_ready() && i.error().is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_hosts_become_file_names() {
        assert!(file_of("www.etsy.com").is_some());
        assert!(file_of("../../etc/passwd").is_none());
        assert!(file_of("a/b").is_none());
        assert!(file_of("").is_none());
    }
}
