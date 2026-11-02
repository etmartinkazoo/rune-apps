//! Mail HTML as a browser lays it out: Blitz (stylo for CSS, taffy for layout, parley for
//! text) on a thread of its own, painted into tiles the reader shows as bitmaps. The UI thread
//! never styles, lays out or shapes a message; it shows pixels, and asks this thread what's
//! under the pointer.
//!
//! Nothing is fetched from here. Images come from the app, which applies its privacy rules
//! (inline ones always, remote ones only when allowed, trackers never); a page asking for one
//! that isn't here yet says so, and it's painted in when it arrives. Remote stylesheets and
//! fonts never load.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, UserAgentStylesheet, local_name};
use blitz_html::HtmlDocument;
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use bytes::Bytes;
use rune::{Pixels, Waker};

/// A tile's side, in device pixels: four to one of Rune's 1024-pixel atlas pages, which it
/// reuses. A tile over 1024 gets a texture of its own each time it shows, some 15 ms a frame.
/// Tiles paint as they come into view.
const TILE_PX: f32 = 512.0;
/// Rows of tiles painted with the layout, before anything asks: a screen's worth at 2x.
const FIRST_ROWS: usize = 4;
/// Laid-out messages kept on the thread, for going back to one or a change of width.
const KEPT: usize = 12;
/// Image bytes kept for pages laid out again (a new width, dark mode toggled).
const IMAGE_BYTES: usize = 96 * 1024 * 1024;
/// What a message's URLs resolve against. It has to be able to be a base: Blitz panics on a
/// relative URL it can't resolve (`about:blank` can't), and mail has them, such as Google's
/// `//fonts.googleapis.com/...`. Protocol-relative ones become https, as other mail clients
/// read them; plain relative ones land on this host, which never resolves, and are dropped.
const BASE_URL: &str = "https://message.invalid/";

/// How a message is laid out and painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The reader column's width, in points.
    pub width: u32,
    /// Device pixels per point.
    pub scale: f32,
    /// Text and link colours for a message that sets none of its own, shown on the app's dark
    /// background. None paints it as written.
    pub ink: Option<([u8; 4], [u8; 4])>,
}

impl Look {
    /// A tile's side, in points.
    pub fn tile(&self) -> f32 {
        (TILE_PX / self.scale.max(0.5)).floor()
    }

    /// Tiles across the column.
    pub fn cols(&self) -> usize {
        (self.width as f32 / self.tile()).ceil().max(1.0) as usize
    }
}

/// A message as the reader shows it.
pub struct Page {
    pub look: Look,
    /// In points; 0 until laid out.
    pub height: f32,
    /// Row by row, left to right, `look.tile()` square (those at the right and bottom edges
    /// less); blank until painted.
    pub tiles: Vec<Pixels>,
    /// The link under the pointer.
    pub hover: Option<String>,
    /// Tiles asked to paint.
    asked: HashSet<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    Hover,
    Click,
    SelectFrom,
    SelectTo,
}

enum Job {
    Show { uid: String, html: Arc<str>, look: Look },
    Paint { uid: String, tiles: Vec<usize> },
    Image { src: String, bytes: Bytes },
    Pointer { uid: String, x: f32, y: f32, kind: Pointer },
    Copy { uid: String },
    Forget { uid: String },
}

impl Job {
    /// The message the job is about, if it's about one.
    fn uid(&self) -> Option<&str> {
        match self {
            Job::Show { uid, .. } | Job::Paint { uid, .. } | Job::Pointer { uid, .. } | Job::Copy { uid } | Job::Forget { uid } => Some(uid),
            Job::Image { .. } => None,
        }
    }
}

/// What the thread answers that the app acts on.
pub enum Done {
    Click { href: String },
    Copied(String),
    /// Images a page asked for that aren't here: CSS backgrounds among them.
    Wants { uid: String, srcs: Vec<String> },
    Laid { uid: String, look: Look, height: f32, tiles: Vec<Pixels> },
    Hover { uid: String, href: Option<String> },
    /// Laying out or painting the page panicked; the reader shows the message's text instead.
    Failed { uid: String },
}

/// The UI's side: what's asked, and what's laid out.
pub struct Pages {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    waker: Arc<Mutex<Option<Waker>>>,
    pub pages: HashMap<String, Page>,
    /// What each page was last asked to look like, and how long its HTML was.
    asked: HashMap<String, (Look, usize)>,
    /// The page a selection was last made in, for Copy.
    pub selected: Option<String>,
    /// Messages whose HTML couldn't be laid out, shown as text.
    pub failed: HashSet<String>,
}

impl Default for Pages {
    fn default() -> Self {
        Pages::new()
    }
}

impl Pages {
    pub fn new() -> Pages {
        let (jobs, rx) = channel();
        let (tx, done) = channel();
        let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
        let wake = waker.clone();
        std::thread::Builder::new()
            .name("pages".into())
            .spawn(move || Thread::new(tx, wake).run(rx))
            .expect("spawn the page thread");
        Pages { jobs, done, waker, pages: HashMap::new(), asked: HashMap::new(), selected: None, failed: HashSet::new() }
    }

    /// Wakes the UI when the thread has an answer.
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(waker);
    }

    /// Lays out `html` for message `uid` as `look`, unless that's already asked.
    pub fn show(&mut self, uid: &str, html: &str, look: Look) {
        if self.asked.get(uid) == Some(&(look, html.len())) {
            return;
        }
        self.asked.insert(uid.to_owned(), (look, html.len()));
        let _ = self.jobs.send(Job::Show { uid: uid.to_owned(), html: Arc::from(html), look });
    }

    /// Tile `i` of `uid` came into view.
    pub fn need(&mut self, uid: &str, i: usize) {
        let Some(page) = self.pages.get_mut(uid) else { return };
        if i < page.tiles.len() && page.asked.insert(i) {
            let _ = self.jobs.send(Job::Paint { uid: uid.to_owned(), tiles: vec![i] });
        }
    }

    pub fn image(&self, src: String, bytes: Bytes) {
        let _ = self.jobs.send(Job::Image { src, bytes });
    }

    /// The pointer at (`x`, `y`) points from the page's top-left.
    pub fn pointer(&mut self, uid: &str, x: f32, y: f32, kind: Pointer) {
        if matches!(kind, Pointer::SelectFrom | Pointer::SelectTo) {
            self.selected = Some(uid.to_owned());
        }
        let _ = self.jobs.send(Job::Pointer { uid: uid.to_owned(), x, y, kind });
    }

    /// Asks for the selected text; it comes back as `Done::Copied`.
    pub fn copy(&self) -> bool {
        let Some(uid) = &self.selected else { return false };
        let _ = self.jobs.send(Job::Copy { uid: uid.clone() });
        true
    }

    /// Keeps only the pages of `wanted`: the rest's tiles go.
    pub fn keep(&mut self, wanted: &HashSet<&str>) {
        let gone: Vec<String> = self.asked.keys().filter(|u| !wanted.contains(u.as_str())).cloned().collect();
        for uid in gone {
            self.asked.remove(&uid);
            self.pages.remove(&uid);
            self.failed.remove(&uid);
            let _ = self.jobs.send(Job::Forget { uid });
        }
    }

    /// Takes in what the thread finished: whether anything did, and what the app acts on.
    pub fn poll(&mut self) -> (bool, Vec<Done>) {
        let mut out = Vec::new();
        let mut any = false;
        for done in self.done.try_iter() {
            any = true;
            match done {
                Done::Laid { uid, look, height, tiles } => {
                    // A layout for a look since replaced is dropped; the newer one is coming.
                    if self.asked.get(&uid).is_some_and(|(l, _)| *l == look) {
                        let asked = (0..(FIRST_ROWS * look.cols()).min(tiles.len())).collect();
                        self.pages.insert(uid, Page { look, height, tiles, hover: None, asked });
                    }
                }
                Done::Hover { uid, href } => {
                    if let Some(p) = self.pages.get_mut(&uid) {
                        p.hover = href;
                    }
                }
                Done::Failed { uid } => {
                    self.pages.remove(&uid);
                    self.failed.insert(uid);
                }
                other => out.push(other),
            }
        }
        (any, out)
    }
}

/// Serves pages their images: `data:` URLs from themselves, anything else from what the app
/// fetched. What isn't here waits, and the app is told it's wanted.
#[derive(Default)]
struct Net {
    bytes: Mutex<ImageBytes>,
    /// Requests waiting for bytes, by URL, with the document that asked.
    waiting: Mutex<HashMap<String, Vec<Waiting>>>,
    /// URLs asked for and not here, by document, since last looked at.
    wanted: Mutex<Vec<(usize, String)>>,
}

#[derive(Default)]
struct ImageBytes {
    map: HashMap<String, Bytes>,
    order: VecDeque<String>,
    size: usize,
}

impl ImageBytes {
    fn insert(&mut self, src: String, bytes: Bytes) {
        self.size += bytes.len();
        if let Some(old) = self.map.insert(src.clone(), bytes) {
            self.size -= old.len();
        } else {
            self.order.push_back(src);
        }
        while self.size > IMAGE_BYTES {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(b) = self.map.remove(&old) {
                self.size -= b.len();
            }
        }
    }
}

impl NetProvider for Net {
    fn fetch(&self, doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        if url.starts_with("data:") {
            if let Some(bytes) = data_url(&url) {
                handler.bytes(url, bytes);
            }
            return;
        }
        if !(url.starts_with("blob:") || url.starts_with("http://") || url.starts_with("https://")) || url.starts_with(BASE_URL) {
            return;
        }
        let have = self.bytes.lock().unwrap_or_else(|e| e.into_inner()).map.get(&url).cloned();
        match have {
            Some(bytes) => handler.bytes(url, bytes),
            None => {
                self.wanted.lock().unwrap_or_else(|e| e.into_inner()).push((doc_id, url.clone()));
                self.waiting.lock().unwrap_or_else(|e| e.into_inner()).entry(url).or_default().push((doc_id, handler));
            }
        }
    }

    /// Blitz asks this only to decide whether a `<head>` stylesheet blocks rendering until it
    /// loads. Remote stylesheets never load here, so one would leave the page blank (1 pt tall)
    /// forever: say so, and Blitz lays the page out without it. Images are fetched as before.
    fn is_noop(&self) -> bool {
        true
    }
}

/// A request waiting for an image, and the document that made it.
type Waiting = (usize, Box<dyn NetHandler>);

/// The bytes of a `data:` URL, base64 or not.
fn data_url(url: &str) -> Option<Bytes> {
    use base64::Engine;
    let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
    if head.ends_with(";base64") {
        let clean: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD.decode(clean).ok().map(Bytes::from)
    } else {
        Some(Bytes::from(url::form_urlencoded::parse(data.as_bytes()).map(|(k, _)| k.into_owned()).collect::<String>()))
    }
}

/// A laid-out message on the thread.
struct Kept {
    html: Arc<str>,
    look: Look,
    doc: HtmlDocument,
    /// The page's width and height as laid out, in CSS pixels.
    size: (f32, f32),
    /// Points per CSS pixel: under 1 when the message is wider than the column and shown
    /// shrunk to fit it, as phones show mail.
    zoom: f32,
    tiles: Vec<Pixels>,
    painted: Vec<bool>,
    /// Where the selection was started and dragged to, as heights down the page in points:
    /// what a drag changes lies between where it was and where it's dragged, so only those
    /// tiles need painting again.
    sel: Option<(f32, f32)>,
}

impl Kept {
    /// Height in points.
    fn height(&self) -> f32 {
        self.size.1 * self.zoom
    }

    fn paint(&mut self, i: usize) {
        let Some((w, h, rgba)) = self.render(i) else { return };
        self.tiles[i].set(w, h, rgba);
        self.painted[i] = true;
    }

    /// Tile `i`'s pixels: width, height, straight-alpha RGBA.
    fn render(&mut self, i: usize) -> Option<(u32, u32, Vec<u8>)> {
        if i >= self.tiles.len() {
            return None;
        }
        let (scale, tile, cols) = (self.look.scale, self.look.tile(), self.look.cols());
        let (left, top) = ((i % cols) as f32 * tile, (i / cols) as f32 * tile);
        let w = ((self.look.width as f32 - left).min(tile) * scale).round().max(1.0) as u32;
        let h = ((self.height() - top).min(tile) * scale).round().max(1.0) as u32;
        let zoomed = f64::from(scale * self.zoom);
        let doc: &mut BaseDocument = &mut self.doc;
        // A tile is the page scrolled to it: paint_scene's own offsets move what's painted
        // further into the surface rather than bringing it up to the corner.
        doc.set_viewport_scroll(blitz_dom::Point { x: f64::from(left / self.zoom), y: f64::from(top / self.zoom) });
        let mut rgba = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |s| blitz_paint::paint_scene(s, doc, zoomed, w, h, 0, 0),
            w,
            h,
        );
        doc.set_viewport_scroll(blitz_dom::Point { x: 0.0, y: 0.0 });
        unpremultiply(&mut rgba);
        Some((w, h, rgba))
    }

    /// The point (`x`, `y`), in points from the page's top-left, in CSS pixels.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.zoom, y / self.zoom)
    }
}

/// Vello paints premultiplied alpha; Rune takes it straight.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8;
            }
        }
    }
}

struct Thread {
    out: Sender<Done>,
    waker: Arc<Mutex<Option<Waker>>>,
    net: Arc<Net>,
    kept: HashMap<String, Kept>,
    order: VecDeque<String>,
}

impl Thread {
    fn new(out: Sender<Done>, waker: Arc<Mutex<Option<Waker>>>) -> Thread {
        Thread { out, waker, net: Arc::default(), kept: HashMap::new(), order: VecDeque::new() }
    }

    fn send(&self, done: Done) {
        let _ = self.out.send(done);
        if let Some(wake) = self.waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            wake();
        }
    }

    fn run(mut self, rx: Receiver<Job>) {
        let mut queue: VecDeque<Job> = VecDeque::new();
        loop {
            queue.extend(rx.try_iter());
            let job = match queue.pop_front() {
                Some(j) => j,
                None => match rx.recv() {
                    Ok(j) => j,
                    Err(_) => return,
                },
            };
            // Only the newest hover, drag of a selection and layout of a message matter: a drag
            // sends a point per pointer move, faster than a long message repaints.
            let superseded = match &job {
                Job::Pointer { uid, kind: kind @ (Pointer::Hover | Pointer::SelectTo), .. } => {
                    queue.extend(rx.try_iter());
                    queue.iter().any(|j| matches!(j, Job::Pointer { uid: u, kind: k, .. } if u == uid && k == kind))
                }
                Job::Show { uid, .. } => {
                    queue.extend(rx.try_iter());
                    queue.iter().any(|j| matches!(j, Job::Show { uid: u, .. } if u == uid))
                }
                _ => false,
            };
            if !superseded {
                // A panic in Blitz costs the one message, not the thread: without this, every
                // HTML message after it would stay blank until the app restarts.
                let uid = job.uid().map(str::to_owned);
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(job))).is_err()
                    && let Some(uid) = uid
                {
                    self.kept.remove(&uid);
                    self.order.retain(|u| *u != uid);
                    self.send(Done::Failed { uid });
                }
            }
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Show { uid, html, look } => self.show(uid, html, look),
            Job::Paint { uid, tiles } => {
                if let Some(k) = self.kept.get_mut(&uid) {
                    for i in tiles {
                        if !k.painted.get(i).copied().unwrap_or(true) {
                            k.paint(i);
                        }
                    }
                }
            }
            Job::Image { src, bytes } => self.image(src, bytes),
            Job::Pointer { uid, x, y, kind } => self.pointer(&uid, x, y, kind),
            Job::Copy { uid } => {
                if let Some(text) = self.kept.get(&uid).and_then(|k| k.doc.get_selected_text()) {
                    self.send(Done::Copied(text));
                }
            }
            Job::Forget { uid } => {
                // The layout stays (KEPT of them) for coming back; the tiles go.
                if let Some(k) = self.kept.get_mut(&uid) {
                    for (tile, painted) in k.tiles.iter().zip(&mut k.painted) {
                        if *painted {
                            tile.set(1, 1, vec![0; 4]);
                            *painted = false;
                        }
                    }
                }
            }
        }
    }

    fn show(&mut self, uid: String, html: Arc<str>, look: Look) {
        let same_html = self.kept.get(&uid).is_some_and(|k| k.html == html && k.look.ink == look.ink);
        if same_html {
            let k = self.kept.get_mut(&uid).expect("checked above");
            if k.look != look {
                k.look = look;
                lay_out(k);
            }
        } else {
            let k = self.build(&html, look);
            self.kept.insert(uid.clone(), k);
            self.order.retain(|u| *u != uid);
            self.order.push_back(uid.clone());
            while self.order.len() > KEPT {
                if let Some(old) = self.order.pop_front() {
                    self.kept.remove(&old);
                }
            }
        }
        self.publish(&uid);
        self.report_wanted();
    }

    /// Fresh tiles for a page just laid out, its first ones painted.
    fn publish(&mut self, uid: &str) {
        let Some(k) = self.kept.get_mut(uid) else { return };
        let cols = k.look.cols();
        let count = (k.height() / k.look.tile()).ceil().max(1.0) as usize * cols;
        k.tiles = (0..count).map(|_| Pixels::new()).collect();
        k.painted = vec![false; count];
        for i in 0..(FIRST_ROWS * cols).min(count) {
            k.paint(i);
        }
        let done = Done::Laid { uid: uid.to_owned(), look: k.look, height: k.height(), tiles: k.tiles.clone() };
        self.send(done);
    }

    fn build(&self, html: &Arc<str>, look: Look) -> Kept {
        let mut sheets = vec![UserAgentStylesheet::default_sheet(), UserAgentStylesheet::parse(MAIL_CSS)];
        if let Some((text, link)) = look.ink {
            sheets.push(UserAgentStylesheet::parse(&format!(
                ":root {{ color: {}; }} a:link, a:visited {{ color: {}; }}",
                css_color(text),
                css_color(link)
            )));
        }
        let config = DocumentConfig {
            viewport: Some(viewport(look.width as f32, 800.0, look, 1.0)),
            base_url: Some(BASE_URL.into()),
            net_provider: Some(self.net.clone()),
            ua_stylesheets: sheets,
            ..Default::default()
        };
        let mut doc = HtmlDocument::from_html(&standard_doctype(html), config);
        legacy_attributes(&mut doc);
        let mut k = Kept { html: html.clone(), look, doc, size: (0.0, 0.0), zoom: 1.0, tiles: Vec::new(), painted: Vec::new(), sel: None };
        lay_out(&mut k);
        k
    }

    fn image(&mut self, src: String, bytes: Bytes) {
        self.net.bytes.lock().unwrap_or_else(|e| e.into_inner()).insert(src.clone(), bytes.clone());
        let waiting = self.net.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&src).unwrap_or_default();
        let mut docs = HashSet::new();
        for (doc, handler) in waiting {
            handler.bytes(src.clone(), bytes.clone());
            docs.insert(doc);
        }
        let uids: Vec<String> = self.kept.iter().filter(|(_, k)| docs.contains(&k.doc.id())).map(|(u, _)| u.clone()).collect();
        for uid in uids {
            let Some(k) = self.kept.get_mut(&uid) else { continue };
            k.doc.handle_messages();
            let before = k.height();
            lay_out(k);
            if (k.height() - before).abs() > 0.5 {
                // Taller or shorter: new tiles.
                self.publish(&uid);
            } else {
                let painted: Vec<usize> = (0..k.painted.len()).filter(|&i| k.painted[i]).collect();
                for i in painted {
                    k.paint(i);
                }
            }
        }
    }

    fn pointer(&mut self, uid: &str, x: f32, y: f32, kind: Pointer) {
        let Some(k) = self.kept.get_mut(uid) else { return };
        let (x, y) = k.at(x, y);
        match kind {
            Pointer::Hover => {
                let href = link_at(&k.doc, x, y);
                self.send(Done::Hover { uid: uid.to_owned(), href });
            }
            Pointer::Click => {
                let had = k.doc.get_selected_text().is_some_and(|t| !t.is_empty());
                k.doc.clear_text_selection();
                if had {
                    repaint_selection(k);
                }
                k.sel = None;
                if let Some(href) = link_at(&k.doc, x, y) {
                    self.send(Done::Click { href });
                }
            }
            Pointer::SelectFrom => {
                if let Some((node, offset)) = k.doc.find_text_position(x, y) {
                    k.doc.set_text_selection(node, offset, node, offset);
                    // A new drag replaces the last selection: its highlight goes.
                    if k.sel.is_some() {
                        repaint_selection(k);
                    }
                    k.sel = Some((y * k.zoom, y * k.zoom));
                }
            }
            Pointer::SelectTo => {
                if k.doc.extend_text_selection_to_point(x, y) {
                    let to = y * k.zoom;
                    match k.sel {
                        Some((from, was)) => {
                            repaint_between(k, was.min(to), was.max(to));
                            k.sel = Some((from, to));
                        }
                        None => repaint(k),
                    }
                }
            }
        }
    }

    fn report_wanted(&self) {
        let wanted = std::mem::take(&mut *self.net.wanted.lock().unwrap_or_else(|e| e.into_inner()));
        let mut by_uid: HashMap<String, Vec<String>> = HashMap::new();
        for (doc, src) in wanted {
            if let Some((uid, _)) = self.kept.iter().find(|(_, k)| k.doc.id() == doc) {
                by_uid.entry(uid.clone()).or_default().push(src);
            }
        }
        for (uid, srcs) in by_uid {
            self.send(Done::Wants { uid, srcs });
        }
    }
}

/// Repaints what's painted of a page (a selection changed).
fn repaint(k: &mut Kept) {
    let painted: Vec<usize> = (0..k.painted.len()).filter(|&i| k.painted[i]).collect();
    for i in painted {
        k.paint(i);
    }
}

/// Repaints the painted tiles of rows reaching from `top` to `bottom` points down the page
/// (and a line either side, for the lines those points fall in).
fn repaint_between(k: &mut Kept, top: f32, bottom: f32) {
    const LINE: f32 = 40.0;
    let (tile, cols) = (k.look.tile(), k.look.cols());
    let (first, last) = (((top - LINE) / tile).floor().max(0.0) as usize, ((bottom + LINE) / tile).floor().max(0.0) as usize);
    let tiles: Vec<usize> = (first * cols..(last + 1) * cols).filter(|&i| k.painted.get(i).copied().unwrap_or(false)).collect();
    for i in tiles {
        k.paint(i);
    }
}

/// Repaints where the selection is (it's been cleared or replaced); everything painted if
/// where it is isn't known.
fn repaint_selection(k: &mut Kept) {
    match k.sel {
        Some((a, b)) => repaint_between(k, a.min(b), a.max(b)),
        None => repaint(k),
    }
}

/// A viewport `width` by `height` CSS pixels, painted at `zoom` of the look's scale.
fn viewport(width: f32, height: f32, look: Look, zoom: f32) -> Viewport {
    let scheme = if look.ink.is_some() { ColorScheme::Dark } else { ColorScheme::Light };
    let scale = look.scale * zoom;
    Viewport::new((width * scale).round() as u32, (height * scale).round().max(1.0) as u32, scale, scheme)
}

/// Lays the page out at its look's width; one wider than that (a fixed 600 px table in a
/// narrower column) is laid out at its own width and shown shrunk to fit.
fn lay_out(k: &mut Kept) {
    let column = k.look.width as f32;
    k.doc.set_viewport(viewport(column, 800.0, k.look, 1.0));
    k.doc.resolve(0.0);
    k.doc.handle_messages();
    k.doc.resolve(0.0);
    let root = k.doc.root_element();
    let (mut w, mut h) = (root.scroll_width(), root.scroll_height());
    let mut width = column;
    if w > column + 1.0 {
        width = w;
        k.doc.set_viewport(viewport(width, h, k.look, 1.0));
        k.doc.resolve(0.0);
        let root = k.doc.root_element();
        (w, h) = (root.scroll_width(), root.scroll_height());
    }
    k.zoom = column / width.max(w).max(1.0);
    // The viewport as tall as the page, so its background and `vh` cover all of it; and at the
    // scale it's painted at. Blitz shapes text at the viewport's scale, so a page shrunk to fit
    // only when painted has its boxes and images shrunk but its text not: lines run off the
    // edge and over the ones below.
    k.doc.set_viewport(viewport(width, h.max(1.0), k.look, k.zoom));
    k.doc.resolve(0.0);
    h = k.doc.root_element().scroll_height();
    k.size = (width, h.max(1.0));
}

/// What a browser draws under (`x`, `y`) in CSS pixels, if it's in a link: its href.
fn link_at(doc: &BaseDocument, x: f32, y: f32) -> Option<String> {
    let hit = doc.hit(x, y)?;
    let mut id = Some(hit.node_id);
    while let Some(node) = id.and_then(|i| doc.get_node(i)) {
        if let Some(el) = node.element_data()
            && el.name.local == local_name!("a")
            && let Some(href) = el.attr(local_name!("href"))
        {
            return Some(href.trim().to_owned());
        }
        id = node.parent;
    }
    None
}

fn css_color(c: [u8; 4]) -> String {
    format!("rgba({}, {}, {}, {})", c[0], c[1], c[2], f32::from(c[3]) / 255.0)
}

/// What mail clients do that browsers don't: sans-serif at the size the reader's plain text is,
/// unless the message says otherwise; no page margin; long words wrap; and the presentational
/// attributes mail still uses.
const MAIL_CSS: &str = "
html { font-family: sans-serif; font-size: 13.5px; }
body { margin: 0; overflow-wrap: break-word; }
table[align=center i] { margin-left: auto; margin-right: auto; }
td[valign=top i], th[valign=top i], tr[valign=top i] > td { vertical-align: top; }
td[valign=middle i], th[valign=middle i], tr[valign=middle i] > td { vertical-align: middle; }
td[valign=bottom i], th[valign=bottom i], tr[valign=bottom i] > td { vertical-align: bottom; }
img { max-width: 100% !important; }
tr > td, tr > th { display: table-cell !important; }
blockquote { margin: 0 0 0 4px; padding-left: 12px; border-left: 2px solid rgba(128, 128, 128, 0.45); }
";

/// `html` with any doctype it declares made the standard one. Older ones (XHTML 1.0
/// Transitional, common in mail) put the parser in limited-quirks mode, where Blitz leaves out
/// its default stylesheet: the head's title and CSS show as text, and paragraphs run together.
/// Mail without a doctype keeps the quirks mode its tables are written for.
fn standard_doctype(html: &str) -> std::borrow::Cow<'_, str> {
    let start = html.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    if start.len() > 9 && start[..9].eq_ignore_ascii_case("<!doctype") {
        let end = start.find('>').map_or(start.len(), |i| i + 1);
        return format!("<!DOCTYPE html>{}", &start[end..]).into();
    }
    html.into()
}

/// Turns presentational attributes Blitz doesn't apply into the inline styles they mean,
/// without overriding a style the message sets itself.
fn legacy_attributes(doc: &mut HtmlDocument) {
    let mut found: Vec<(NodeId, &'static str, String)> = Vec::new();
    let mut stack = vec![(doc.root_element().id, None::<(String, String)>)];
    while let Some((id, table)) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        let mut inner = table.clone();
        if let Some(el) = node.element_data() {
            let has = |prop: &str| el.attr(local_name!("style")).is_some_and(|s| s.to_ascii_lowercase().contains(prop));
            let attr = |name| el.attr(name).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
            match &*el.name.local {
                // Never drawn, whatever a message's own CSS does to the cascade.
                "head" | "title" | "style" | "script" | "template" | "noscript" | "meta" | "link" => {
                    found.push((id, "display", "none".into()));
                }
                "table" => {
                    let pad = attr(local_name!("cellpadding")).and_then(|v| px(&v)).unwrap_or_default();
                    let space = attr(local_name!("cellspacing")).and_then(|v| px(&v));
                    if let Some(s) = space
                        && !has("border-spacing")
                    {
                        found.push((id, "border-spacing", s));
                    }
                    inner = Some((pad, String::new()));
                }
                "td" | "th" => {
                    if let Some((pad, _)) = &table
                        && !pad.is_empty()
                        && !has("padding")
                    {
                        found.push((id, "padding", pad.clone()));
                    }
                    if let Some(h) = attr(local_name!("height")).and_then(|v| px(&v))
                        && !has("height")
                    {
                        found.push((id, "height", h));
                    }
                }
                "font" => {
                    if let Some(c) = attr(local_name!("color"))
                        && !has("color")
                    {
                        found.push((id, "color", c));
                    }
                    if let Some(f) = attr(local_name!("face"))
                        && !has("font-family")
                    {
                        found.push((id, "font-family", f));
                    }
                    if let Some(size) = attr(local_name!("size")).and_then(|v| font_size(&v))
                        && !has("font-size")
                    {
                        found.push((id, "font-size", size.into()));
                    }
                }
                _ => {}
            }
            // Blitz reads a hex bgcolor but not a named one ("white"); CSS takes either.
            if let Some(c) = attr(local_name!("bgcolor")).filter(|c| c.chars().all(|ch| ch.is_ascii_alphabetic()))
                && !has("background")
            {
                found.push((id, "background-color", c));
            }
            if let Some(bg) = attr(local_name!("background"))
                && !has("background")
            {
                found.push((id, "background-image", format!("url(\"{}\")", bg.replace('"', "%22"))));
            }
        }
        for &child in node.children.iter().rev() {
            stack.push((child, inner.clone()));
        }
    }
    let mut m = doc.mutate();
    for (id, prop, value) in found {
        m.set_style_property(id, prop, &value);
    }
}

/// An HTML length attribute ("10", "10px") as CSS.
fn px(v: &str) -> Option<String> {
    let n: f32 = v.trim_end_matches("px").trim().parse().ok().filter(|n: &f32| n.is_finite() && *n >= 0.0)?;
    Some(format!("{n}px"))
}

/// `<font size>`: 1 to 7, or relative to 3.
fn font_size(v: &str) -> Option<&'static str> {
    const SIZES: [&str; 7] = ["x-small", "small", "medium", "large", "x-large", "xx-large", "xxx-large"];
    let n: i32 = match v.trim() {
        r if r.starts_with('+') || r.starts_with('-') => 3 + r.parse::<i32>().ok()?,
        a => a.parse().ok()?,
    };
    Some(SIZES[(n.clamp(1, 7) - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOK: Look = Look { width: 400, scale: 1.0, ink: None };

    fn thread() -> (Thread, Receiver<Done>) {
        let (tx, rx) = channel();
        (Thread::new(tx, Arc::default()), rx)
    }

    fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn lays_out_paints_and_finds_links() {
        let (mut t, rx) = thread();
        let html = r##"<table width="100%" cellpadding="20" bgcolor="#123456"><tr><td>
            <a href="https://example.com/x" style="display:block;height:60px">Go</a></td></tr></table>
            <p style="height:900px">tall</p>"##;
        t.show("m".into(), Arc::from(html), LOOK);
        let Some(Done::Laid { height, tiles, .. }) = rx.try_iter().find(|d| matches!(d, Done::Laid { .. })) else { panic!("laid out") };
        assert!(height > 900.0, "{height}");
        assert_eq!(LOOK.tile(), 512.0);
        assert_eq!(tiles.len(), (height / 512.0).ceil() as usize, "one column: 400 points fit a tile");
        assert_eq!(tiles[0].size(), (400, 512), "the first tiles are painted with the layout");
        let k = t.kept.get_mut("m").unwrap();
        let (w, _, rgba) = k.render(0).unwrap();
        assert_eq!(pixel(&rgba, w, 5, 5), [0x12, 0x34, 0x56, 255], "bgcolor painted");
        assert_eq!(link_at(&k.doc, 30.0, 30.0).as_deref(), Some("https://example.com/x"), "cellpadding put the link 20 px in");
        assert_eq!(link_at(&k.doc, 5.0, 5.0), None);
    }

    #[test]
    fn each_tile_paints_its_own_part_of_the_page() {
        let (mut t, _rx) = thread();
        // Red top-left, blue to the right of the first column, green below the first row.
        let html = r#"<body style="margin:0"><div style="display:flex;height:600px">
            <div style="width:520px;background:#ff0000"></div><div style="width:280px;background:#0000ff"></div></div>
            <div style="height:400px;background:#00ff00"></div></body>"#;
        t.show("t".into(), Arc::from(html), Look { width: 800, ..LOOK });
        let k = t.kept.get_mut("t").unwrap();
        assert_eq!(k.look.cols(), 2);
        let at = |k: &mut Kept, i: usize, x: u32, y: u32| {
            let (w, _, rgba) = k.render(i).unwrap();
            pixel(&rgba, w, x, y)
        };
        assert_eq!(at(k, 0, 5, 5), [255, 0, 0, 255]);
        assert_eq!(at(k, 1, 20, 5), [0, 0, 255, 255], "x 532: the blue column");
        assert_eq!(at(k, 2, 5, 100), [0, 255, 0, 255], "y 612: the green band");
        assert_eq!(link_at(&k.doc, 5.0, 5.0), None, "hit-testing isn't left scrolled");
    }

    #[test]
    fn an_old_doctype_keeps_the_default_styles() {
        let (mut t, _rx) = thread();
        let html = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd">
            <html><head><title>TITLE</title></head><body style="margin:0"><p style="margin:0">one</p><p style="margin:0">two</p></body></html>"#;
        t.show("d".into(), Arc::from(html), LOOK);
        assert!(t.kept["d"].height() > 30.0, "two paragraphs, one above the other: {}", t.kept["d"].height());
        assert_eq!(standard_doctype("<p>x</p>"), "<p>x</p>", "none stays none");
    }

    #[test]
    fn relative_urls_lay_out_and_protocol_relative_ones_are_https() {
        // Google's security alerts link `//fonts.googleapis.com/...`: Blitz panicked resolving
        // it against `about:blank`, and the page never showed.
        let (mut t, rx) = thread();
        let html = r#"<!DOCTYPE html><html><head><link href="//fonts.googleapis.com/css?family=Google+Sans" rel="stylesheet">
            </head><body><img src="//example.com/a.png" width="10" height="10"><img src="logo.png" width="10" height="10">
            <p>A new sign-in</p></body></html>"#;
        t.show("m".into(), Arc::from(html), LOOK);
        t.report_wanted();
        let done: Vec<Done> = rx.try_iter().collect();
        let heights: Vec<f32> = done.iter().filter_map(|d| if let Done::Laid { height, .. } = d { Some(*height) } else { None }).collect();
        assert!(heights.iter().any(|h| *h > 10.0), "laid out: {heights:?}");
        let wanted: Vec<&String> = done.iter().filter_map(|d| if let Done::Wants { srcs, .. } = d { Some(srcs) } else { None }).flatten().collect();
        assert!(wanted.iter().any(|s| *s == "https://example.com/a.png"), "{wanted:?}");
        assert!(!wanted.iter().any(|s| s.contains("logo.png")), "a bare relative path points at nothing: {wanted:?}");
    }

    #[test]
    fn a_panic_costs_one_message_not_the_thread() {
        let (tx, rx) = channel();
        let (jobs, job_rx) = channel();
        std::thread::spawn(move || Thread::new(tx, Arc::default()).run(job_rx));
        // A NaN scale makes parley panic breaking lines: a stand-in for whatever Blitz trips on.
        let bad = Look { width: 0, scale: f32::NAN, ink: None };
        jobs.send(Job::Show { uid: "a".into(), html: Arc::from("<p>one</p>"), look: bad }).unwrap();
        jobs.send(Job::Show { uid: "b".into(), html: Arc::from("<p>two</p>"), look: LOOK }).unwrap();
        let mut failed = false;
        let mut laid = false;
        for d in rx.iter() {
            match d {
                Done::Failed { uid } if uid == "a" => failed = true,
                Done::Laid { uid, .. } if uid == "b" => {
                    laid = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(failed, "the message that panicked is reported, to be shown as text");
        assert!(laid, "the thread lived to lay out the next message");
    }

    #[test]
    fn the_head_never_shows() {
        let (mut t, _rx) = thread();
        let html = "<html><head><title>TITLE</title><style>html, body { display: block } head, title, style { display: block }</style></head><body style='margin:0'><p style='margin:0'>x</p></body></html>";
        t.show("h".into(), Arc::from(html), LOOK);
        assert!(t.kept["h"].height() < 40.0, "only the body's line: {}", t.kept["h"].height());
    }

    #[test]
    fn a_wide_message_shrinks_to_the_column() {
        let (mut t, _rx) = thread();
        t.show("w".into(), Arc::from(r#"<table width="800" cellspacing="0" cellpadding="0"><tr><td style="height:400px">wide</td></tr></table>"#), LOOK);
        let k = &t.kept["w"];
        assert!((k.zoom - 0.5).abs() < 0.02, "800 px in a 400 point column: {}", k.zoom);
        assert!((k.height() - 200.0).abs() < 20.0, "half as tall: {}", k.height());
    }

    #[test]
    fn images_wait_for_the_app_and_data_urls_dont() {
        let (mut t, rx) = thread();
        let png = {
            let mut out = Vec::new();
            let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([255, 0, 0, 255]));
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        use base64::Engine;
        let data = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
        let html = format!(
            r#"<body style="margin:0"><img src="{data}" width="10" height="10" style="display:block"><img src="https://img.example/a.png" width="10" height="10" style="display:block"></body>"#
        );
        t.show("i".into(), Arc::from(html.as_str()), LOOK);
        let wants: Vec<String> = rx.try_iter().filter_map(|d| match d { Done::Wants { srcs, .. } => Some(srcs), _ => None }).flatten().collect();
        assert_eq!(wants, ["https://img.example/a.png"], "only the remote one is asked of the app");
        let red = |t: &mut Thread, y: u32| {
            let (w, _, rgba) = t.kept.get_mut("i").unwrap().render(0).unwrap();
            pixel(&rgba, w, 2, y) == [255, 0, 0, 255]
        };
        assert!(red(&mut t, 2), "the data: image painted");
        assert!(!red(&mut t, 12), "the remote one waits");
        t.image("https://img.example/a.png".into(), Bytes::from(png));
        assert!(red(&mut t, 12), "and paints in once the app has it");
    }

    #[test]
    fn dark_ink_colours_a_message_without_its_own() {
        let (mut t, _rx) = thread();
        let look = Look { ink: Some(([250, 250, 250, 255], [0, 128, 255, 255])), ..LOOK };
        t.show("d".into(), Arc::from(r#"<p style="margin:0;font-size:40px;line-height:40px">&#9608;&#9608;&#9608;</p>"#), look);
        let (w, _, rgba) = t.kept.get_mut("d").unwrap().render(0).unwrap();
        assert_eq!(pixel(&rgba, w, 390, 20)[3], 0, "no background: the app's shows through");
        let lit = (0..40).flat_map(|y| (0..40).map(move |x| (x, y))).any(|(x, y)| pixel(&rgba, w, x, y)[..3] == [250, 250, 250]);
        assert!(lit, "text in the light ink");
    }

    #[test]
    fn dragging_selects_text_to_copy() {
        let (mut t, rx) = thread();
        t.show("t".into(), Arc::from("<p style='margin:0;font-size:20px;line-height:20px'>alpha beta gamma</p>"), LOOK);
        t.pointer("t", 1.0, 10.0, Pointer::SelectFrom);
        t.pointer("t", 399.0, 10.0, Pointer::SelectTo);
        t.handle(Job::Copy { uid: "t".into() });
        let copied = rx.try_iter().find_map(|d| match d { Done::Copied(text) => Some(text), _ => None });
        assert_eq!(copied.as_deref(), Some("alpha beta gamma"));
        t.pointer("t", 1.0, 10.0, Pointer::Click);
        assert!(t.kept["t"].doc.get_selected_text().is_none_or(|s| s.is_empty()), "a click clears it");
    }

    /// A cell made a block (`display: block !important`, as responsive mail templates do to
    /// centre a column) stays a cell: Blitz laid it out with no height, so all the message
    /// but its logo row was cut off. A named bgcolor paints, as a hex one does.
    #[test]
    fn a_cell_styled_as_a_block_still_shows() {
        let (mut t, _rx) = thread();
        let html = "<!DOCTYPE html><html><body style='margin:0'><table width='100%' cellpadding='0' cellspacing='0'>\
            <tr><td style='height:40px'>logo</td></tr>\
            <tr><td style='display: block !important; width: 300px; margin: 0 auto'>\
            <table class='main' width='100%' bgcolor='white'><tr><td style='font-size:20px;line-height:20px'>\
            line one<br>line two<br>line three<br>line four<br>line five</td></tr></table>\
            </td></tr></table></body></html>";
        t.show("b".into(), Arc::from(html), LOOK);
        let k = &t.kept["b"];
        assert!(k.height() >= 140.0, "the logo row and five lines: {}", k.height());
        let (w, _, rgba) = t.kept.get_mut("b").unwrap().render(0).unwrap();
        assert_eq!(pixel(&rgba, w, 200, 60)[..3], [255, 255, 255], "bgcolor=white paints white");
    }

    /// How many times a tile has been painted.
    fn paints(tile: &Pixels) -> u64 {
        let debug = format!("{tile:?}");
        debug.rsplit("gen ").next().and_then(|n| n.trim_end_matches(')').parse().ok()).expect("Pixels' Debug shows its generation")
    }

    #[test]
    fn selecting_repaints_only_where_it_changes() {
        let (mut t, rx) = thread();
        let para = "<p style='margin:0;font-size:20px;line-height:20px'>alpha beta gamma</p>";
        t.show("t".into(), Arc::from(para.repeat(200)), LOOK);
        let count = t.kept["t"].tiles.len();
        assert!(count >= 6, "a long page: {count} tiles");
        t.handle(Job::Paint { uid: "t".into(), tiles: (0..count).collect() });
        let before: Vec<u64> = t.kept["t"].tiles.iter().map(paints).collect();
        t.pointer("t", 1.0, 10.0, Pointer::SelectFrom);
        for x in [100.0, 200.0, 399.0] {
            t.pointer("t", x, 10.0, Pointer::SelectTo);
        }
        let after: Vec<u64> = t.kept["t"].tiles.iter().map(paints).collect();
        assert!(after[0] > before[0], "the selected line is painted");
        assert_eq!(after[count - 1], before[count - 1], "a tile far from the selection isn't");
        t.handle(Job::Copy { uid: "t".into() });
        let copied = rx.try_iter().find_map(|d| match d { Done::Copied(text) => Some(text), _ => None });
        assert_eq!(copied.as_deref(), Some("alpha beta gamma"));
        t.pointer("t", 1.0, 10.0, Pointer::Click);
        let cleared: Vec<u64> = t.kept["t"].tiles.iter().map(paints).collect();
        assert!(cleared[0] > after[0], "clearing paints the highlight away");
        assert_eq!(cleared[count - 1], before[count - 1]);
    }

    #[test]
    #[ignore = "timing, by hand"]
    fn drag_timing() {
        let (mut t, _rx) = thread();
        let para = "<p style='margin:0;font-size:16px;line-height:20px'>alpha beta gamma delta epsilon zeta eta theta</p>";
        t.show("t".into(), Arc::from(para.repeat(400)), LOOK);
        let count = t.kept["t"].tiles.len();
        t.handle(Job::Paint { uid: "t".into(), tiles: (0..count).collect() });
        let start = std::time::Instant::now();
        t.pointer("t", 1.0, 10.0, Pointer::SelectFrom);
        for i in 0..200 {
            t.pointer("t", 300.0, 10.0 + i as f32 * 2.0, Pointer::SelectTo);
        }
        eprintln!("{count} tiles: 200 drag moves in {:?}", start.elapsed());
    }

    #[test]
    fn svg_images_paint() {
        let (mut t, _rx) = thread();
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#0067b9"/></svg>"##;
        use base64::Engine;
        let src = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg));
        t.show("s".into(), Arc::from(format!(r#"<body style="margin:0"><img src="{src}" width="10" height="10" style="display:block"></body>"#).as_str()), LOOK);
        let (w, _, rgba) = t.kept.get_mut("s").unwrap().render(0).unwrap();
        assert_eq!(pixel(&rgba, w, 5, 5), [0, 0x67, 0xb9, 255]);
    }

    #[test]
    fn a_shrunk_message_shrinks_its_text_too() {
        // Text is shaped at the viewport's scale: laid out at 1x and painted at half, a glyph
        // came out twice the size of its box, over the lines around it.
        let (mut t, _rx) = thread();
        let html = r#"<body style="margin:0"><div style="width:800px"><p style="margin:0;font-size:40px;line-height:40px">&#9608;</p></div></body>"#;
        t.show("z".into(), Arc::from(html), LOOK);
        let k = t.kept.get_mut("z").unwrap();
        assert!((k.zoom - 0.5).abs() < 0.02, "{}", k.zoom);
        let (w, _, rgba) = k.render(0).unwrap();
        let inked = |x, y| pixel(&rgba, w, x, y)[3] > 128;
        assert!((k.height() - 20.0).abs() < 1.0, "{}", k.height());
        assert!(inked(4, 10), "the block glyph, at half size");
        assert!(!inked(30, 10), "and no bigger: 20 points wide, not 40");
    }

    #[test]
    fn an_image_wider_than_the_column_fits_it() {
        // Outlook writes each image's own width as its max-width, which would make the whole
        // message shrink to fit a big screenshot.
        let (mut t, _rx) = thread();
        let html = r#"<body style="margin:0"><img src="https://img.example/s.png" width="1600" height="10" style="max-width:1901px;display:block"><p>text</p></body>"#;
        t.show("i".into(), Arc::from(html), LOOK);
        assert_eq!(t.kept["i"].zoom, 1.0);
    }

    #[test]
    fn font_and_sizes_read_as_css() {
        assert_eq!(font_size("+1"), Some("large"));
        assert_eq!(font_size("9"), Some("xxx-large"));
        assert_eq!(px("10"), Some("10px".into()));
        assert_eq!(px("-1"), None);
        assert_eq!(data_url("data:text/plain,a%20b").as_deref(), Some(&b"a b"[..]));
    }
}


