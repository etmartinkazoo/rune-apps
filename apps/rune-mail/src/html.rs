//! Mail HTML to a small block model the reader draws with Rune: paragraphs of styled runs and
//! links, images, quotes, lists, rules, and tables flattened into rows and boxes. It runs on
//! the network thread, so the UI only builds elements.
//!
//! Mail HTML is mostly nested layout tables with inline styles. This keeps what carries
//! meaning (text, emphasis, colour, links, images, alignment, backgrounds, column widths) and
//! drops what can't be drawn faithfully without a browser (floats, positioning, most CSS).

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};

const MAX_DEPTH: usize = 160;
pub const BASE_SIZE: f32 = 13.5;

pub type Rgba = [u8; 4];

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub mono: bool,
    pub color: Option<Rgba>,
    pub size: Option<f32>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Width {
    Auto,
    Px(f32),
    Percent(f32),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Block {
    Text { text: String, runs: Vec<Run>, align: Align, size: f32 },
    Image { src: String, width: Option<f32>, height: Option<f32>, link: Option<String>, align: Align },
    Rule,
    Pre(String),
    Quote(Vec<Block>),
    List { ordered: bool, items: Vec<Vec<Block>> },
    /// A div or single-column table: a box with an optional background and width limit.
    Group { blocks: Vec<Block>, bg: Option<Rgba>, padding: f32, align: Align, max_width: Option<f32> },
    /// A table row with more than one column.
    Row(Vec<Cell>),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub bg: Option<Rgba>,
    pub width: Width,
    pub padding: f32,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Doc {
    pub blocks: Vec<Block>,
    /// The message sets its own colours, so it's drawn on a light page even in dark mode.
    pub colored: bool,
    /// Some images load from the network (trackable, so off until allowed).
    pub remote_images: bool,
    /// Images dropped as trackers, by service ("Mailchimp", "Tracking pixel"), once each.
    pub trackers: Vec<String>,
    /// Click-tracking services behind links whose destination can't be cleaned, once each.
    pub tracked_links: Vec<String>,
}

impl Doc {
    /// Every image source, in document order.
    pub fn images(&self) -> Vec<&str> {
        fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a str>) {
            for b in blocks {
                match b {
                    Block::Image { src, .. } => out.push(src),
                    Block::Quote(bs) | Block::Group { blocks: bs, .. } => walk(bs, out),
                    Block::List { items, .. } => items.iter().for_each(|i| walk(i, out)),
                    Block::Row(cells) => cells.iter().for_each(|c| walk(&c.blocks, out)),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }
}

/// `html` with each `cid:` reference replaced by what `cid` maps it to (left as is otherwise),
/// so the result can be kept and parsed again without the message's parts.
pub fn resolve_cids(html: &str, cid: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("cid:") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 4..];
        let end = after.find(['"', '\'', ' ', '>', ')']).unwrap_or(after.len());
        match cid(&after[..end]) {
            Some(src) => out.push_str(&src),
            None => out.push_str(&rest[i..i + 4 + end]),
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Parses `html`. `cid` maps a `cid:` image reference to a fetchable source.
pub fn parse(html: &str, cid: &dyn Fn(&str) -> Option<String>) -> Doc {
    if too_deep(html) {
        return parse(&stripped(html), cid);
    }
    let dom = Html::parse_document(html);
    let mut cx = Cx { cid, colored: false, remote: false, trackers: Vec::new(), tracked_links: Vec::new() };
    let mut b = Builder::default();
    let ctx = Ctx { style: Style::default(), align: Align::Start, pre: false, silent: false };
    cx.children(dom.root_element(), &ctx, &mut b, 0);
    let blocks = simplify(b.finish());
    let dedup = |mut v: Vec<String>| {
        v.sort();
        v.dedup();
        v
    };
    Doc {
        blocks,
        colored: cx.colored,
        remote_images: cx.remote,
        trackers: dedup(cx.trackers),
        tracked_links: dedup(cx.tracked_links),
    }
}

/// Opening tags nested this deep make the HTML parser's time grow with the square of the
/// depth (100,000 nested `<div>`s take some 20 s), and no real mail comes close.
const MAX_PARSE_DEPTH: usize = 1000;

/// Whether `html` opens more than [`MAX_PARSE_DEPTH`] elements without closing them. A rough
/// count: tags whose end tag is optional or that have none don't count, so it errs on deep.
fn too_deep(html: &str) -> bool {
    const UNCOUNTED: [&str; 27] = [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr", "p",
        "li", "td", "th", "tr", "dt", "dd", "option", "optgroup", "tbody", "thead", "tfoot", "colgroup", "!",
    ];
    let mut depth = 0usize;
    for tag in html.split('<').skip(1) {
        let (closing, tag) = match tag.strip_prefix('/') {
            Some(t) => (true, t),
            None => (false, tag),
        };
        let name_end = tag.find(|c: char| !c.is_ascii_alphanumeric() && c != '!').unwrap_or(tag.len());
        let name = tag[..name_end].to_ascii_lowercase();
        if name.is_empty() || name.starts_with('!') || UNCOUNTED.contains(&name.as_str()) {
            continue;
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else if !tag.split('>').next().is_some_and(|t| t.ends_with('/')) {
            depth += 1;
            if depth > MAX_PARSE_DEPTH {
                return true;
            }
        }
    }
    false
}

/// The lines of text in `html`, its tags dropped: what a mail too deep to parse shows instead.
fn text_lines(html: &str) -> Vec<String> {
    let mut text = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        text.push_str(&rest[..i]);
        rest = rest[i..].find('>').map_or("", |j| &rest[i + j + 1..]);
        text.push('\n');
    }
    text.push_str(rest);
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

fn stripped(html: &str) -> String {
    text_lines(html).iter().map(|l| format!("<p>{l}</p>")).collect()
}

/// `html` as plain text, for replies and messages without a text part.
pub fn to_text(html: &str) -> String {
    if too_deep(html) {
        return text_lines(html).join("\n");
    }
    html2text::from_read(html.as_bytes(), 100)
}

#[derive(Clone)]
struct Ctx {
    style: Style,
    align: Align,
    pre: bool,
    /// Under `font-size: 0`: text here doesn't show, but what sets its own size again does.
    /// Layout tables use it to close the gaps between columns.
    silent: bool,
}

/// Collects inline text into the current paragraph and finished blocks behind it.
#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    text: String,
    runs: Vec<Run>,
    align: Align,
}

impl Builder {
    fn push(&mut self, s: &str, cx: &Ctx) {
        if cx.silent {
            return;
        }
        if self.text.is_empty() {
            self.align = cx.align;
        }
        let start = self.text.len();
        for ch in s.chars() {
            if cx.pre || !ch.is_whitespace() {
                self.text.push(ch);
            } else if !(self.text.is_empty() || self.text.ends_with([' ', '\n'])) {
                self.text.push(' ');
            }
        }
        self.run(start, &cx.style);
    }

    fn line_break(&mut self, cx: &Ctx) {
        if cx.silent {
            return;
        }
        while self.text.ends_with(' ') {
            self.text.pop();
        }
        let start = self.text.len();
        self.text.push('\n');
        self.run(start, &cx.style);
    }

    fn run(&mut self, start: usize, style: &Style) {
        let end = self.text.len();
        if end == start {
            return;
        }
        match self.runs.last_mut() {
            Some(last) if last.end == start && last.style == *style => last.end = end,
            _ => self.runs.push(Run { start, end, style: style.clone() }),
        }
    }

    fn flush(&mut self) {
        let text = std::mem::take(&mut self.text);
        let mut runs = std::mem::take(&mut self.runs);
        let trimmed = text.trim_end();
        let lead = trimmed.len() - trimmed.trim_start().len();
        let body = trimmed.trim_start();
        if body.is_empty() {
            return;
        }
        runs.retain_mut(|r| {
            r.start = r.start.clamp(lead, trimmed.len()) - lead;
            r.end = r.end.clamp(lead, trimmed.len()) - lead;
            r.end > r.start
        });
        let size = runs.first().and_then(|r| r.style.size).unwrap_or(BASE_SIZE);
        self.blocks.push(Block::Text { text: body.to_owned(), runs, align: self.align, size });
    }

    fn block(&mut self, b: Block) {
        self.flush();
        self.blocks.push(b);
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.blocks
    }
}

struct Cx<'a> {
    cid: &'a dyn Fn(&str) -> Option<String>,
    colored: bool,
    remote: bool,
    trackers: Vec<String>,
    tracked_links: Vec<String>,
}

impl Cx<'_> {
    fn children(&mut self, el: ElementRef, ctx: &Ctx, b: &mut Builder, depth: usize) {
        for child in el.children() {
            self.node(child, ctx, b, depth + 1);
        }
    }

    fn nested(&mut self, el: ElementRef, ctx: &Ctx, depth: usize) -> Vec<Block> {
        let mut b = Builder::default();
        self.children(el, ctx, &mut b, depth);
        b.finish()
    }

    fn node(&mut self, node: NodeRef<Node>, ctx: &Ctx, b: &mut Builder, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match node.value() {
            Node::Text(t) => b.push(t, ctx),
            Node::Element(_) => {
                if let Some(el) = ElementRef::wrap(node) {
                    self.element(el, ctx, b, depth);
                }
            }
            _ => {}
        }
    }

    fn element(&mut self, el: ElementRef, parent: &Ctx, b: &mut Builder, depth: usize) {
        let e = el.value();
        let name = e.name();
        let css = Css::parse(e.attr("style").unwrap_or_default());
        if matches!(name, "head" | "style" | "script" | "title" | "meta" | "link" | "noscript" | "template" | "svg"
            | "iframe" | "object" | "video" | "audio" | "input" | "select" | "textarea")
            || e.attr("hidden").is_some()
            || css.hidden()
        {
            return;
        }
        let mut ctx = parent.clone();
        let st = &mut ctx.style;
        match name {
            "b" | "strong" | "th" => st.bold = true,
            "i" | "em" | "cite" | "var" => st.italic = true,
            "u" | "ins" => st.underline = true,
            "code" | "tt" | "kbd" | "samp" => st.mono = true,
            "small" => st.size = Some(st.size.unwrap_or(BASE_SIZE) * 0.85),
            "big" => st.size = Some(st.size.unwrap_or(BASE_SIZE) * 1.2),
            "a" => {
                if let Some(href) = e.attr("href").map(str::trim).filter(|h| openable(h)) {
                    if let Some(service) = crate::privacy::click_tracker(href) {
                        self.tracked_links.push(service.to_owned());
                    }
                    st.link = Some(href.to_owned());
                    // Links take the link colour unless they set their own, as in a browser.
                    st.color = None;
                }
            }
            "center" => ctx.align = Align::Center,
            _ => {}
        }
        if let Some(h) = heading(name) {
            ctx.style.bold = true;
            ctx.style.size = Some(h);
        }
        if let Some(c) = e.attr("color").and_then(color) {
            ctx.style.color = Some(c);
            self.colored = true;
        }
        if name == "font"
            && let Some(sz) = e.attr("size").and_then(|s| s.trim().parse::<f32>().ok()).filter(|s| s.is_finite())
        {
            ctx.style.size = Some([10.0, 11.5, BASE_SIZE, 16.0, 19.0, 24.0, 30.0][sz.clamp(1.0, 7.0) as usize - 1]);
            ctx.silent = false;
        }
        css.apply(&mut ctx, self);
        // On a table or image, `align` places the element itself (see `table` and `image`).
        if !matches!(name, "table" | "img")
            && let Some(a) = e.attr("align").and_then(align)
        {
            ctx.align = a;
        }
        let bg = css.background.or_else(|| e.attr("bgcolor").and_then(color)).filter(|c| c[3] > 0);
        if bg.is_some() {
            self.colored = true;
        }

        match name {
            "br" => b.line_break(&ctx),
            "hr" => b.block(Block::Rule),
            "img" => self.image(el, &css, &ctx, b),
            "pre" => {
                let text: String = el.text().collect();
                b.block(Block::Pre(text.trim_end().to_owned()));
            }
            "blockquote" => {
                let inner = self.nested(el, &ctx, depth);
                b.block(Block::Quote(inner));
            }
            "ul" | "ol" => {
                let items = el
                    .children()
                    .filter_map(ElementRef::wrap)
                    .map(|li| self.nested(li, &ctx, depth + 1))
                    .filter(|i| !i.is_empty())
                    .collect();
                b.block(Block::List { ordered: name == "ol", items });
            }
            "table" => self.table(el, &ctx, &css, bg, b, depth),
            _ if is_block(name) => {
                let padding = css.padding.unwrap_or(0.0);
                let max_width = css.max_width.or(css.width_px);
                if bg.is_some() || padding > 0.0 || max_width.is_some() {
                    let blocks = self.nested(el, &ctx, depth);
                    let align = if css.auto_margin { Align::Center } else { ctx.align };
                    b.block(Block::Group { blocks, bg, padding, align, max_width });
                } else {
                    b.flush();
                    self.children(el, &ctx, b, depth);
                    b.flush();
                }
            }
            _ => self.children(el, &ctx, b, depth),
        }
    }

    fn image(&mut self, el: ElementRef, css: &Css, ctx: &Ctx, b: &mut Builder) {
        let e = el.value();
        let width = css.width_px.or_else(|| e.attr("width").and_then(px));
        let height = css.height_px.or_else(|| e.attr("height").and_then(px));
        let Some(src) = e.attr("src").map(str::trim) else { return };
        // Spy pixels and open-tracking endpoints never load, even with remote content on.
        if let Some(service) = crate::privacy::image_tracker(src, width, height) {
            self.trackers.push(service.to_owned());
            return;
        }
        let src = if let Some(id) = src.strip_prefix("cid:") {
            match (self.cid)(id) {
                Some(s) => s,
                None => return,
            }
        } else if src.starts_with("http://") || src.starts_with("https://") {
            self.remote = true;
            src.to_owned()
        } else if src.starts_with("data:image/") || src.starts_with("blob:") {
            src.to_owned()
        } else {
            return;
        };
        let align = e.attr("align").and_then(align).unwrap_or(ctx.align);
        b.block(Block::Image { src, width, height, link: ctx.style.link.clone(), align });
    }

    fn table(&mut self, el: ElementRef, ctx: &Ctx, css: &Css, bg: Option<Rgba>, b: &mut Builder, depth: usize) {
        let e = el.value();
        let cellpadding = e.attr("cellpadding").and_then(px).unwrap_or(0.0);
        let rows: Vec<ElementRef> = el
            .children()
            .filter_map(ElementRef::wrap)
            .flat_map(|c| match c.value().name() {
                "thead" | "tbody" | "tfoot" => c.children().filter_map(ElementRef::wrap).collect::<Vec<_>>(),
                _ => vec![c],
            })
            .filter(|r| r.value().name() == "tr")
            .collect();
        let mut blocks = Vec::new();
        for row in rows {
            let mut cells = Vec::new();
            for cell in row.children().filter_map(ElementRef::wrap) {
                let ce = cell.value();
                if !matches!(ce.name(), "td" | "th") {
                    continue;
                }
                let ccss = Css::parse(ce.attr("style").unwrap_or_default());
                if ccss.hidden() {
                    continue;
                }
                let mut cctx = ctx.clone();
                if ce.name() == "th" {
                    cctx.style.bold = true;
                }
                ccss.apply(&mut cctx, self);
                if let Some(a) = ce.attr("align").and_then(align) {
                    cctx.align = a;
                }
                let cbg = ccss.background.or_else(|| ce.attr("bgcolor").and_then(color)).filter(|c| c[3] > 0);
                if cbg.is_some() {
                    self.colored = true;
                }
                let width = match ccss.width_px.or_else(|| ce.attr("width").and_then(px)) {
                    Some(w) => Width::Px(w),
                    None => ce.attr("width").and_then(percent).or(ccss.width_pct).map_or(Width::Auto, Width::Percent),
                };
                let inner = self.nested(cell, &cctx, depth + 1);
                cells.push(Cell { blocks: inner, bg: cbg, width, padding: ccss.padding.unwrap_or(cellpadding) });
            }
            let filled = cells.iter().filter(|c| !c.blocks.is_empty() || c.bg.is_some()).count();
            match filled {
                0 => {}
                1 if cells.len() == 1 || cells.iter().all(|c| c.bg.is_none()) => {
                    let c = cells.into_iter().find(|c| !c.blocks.is_empty() || c.bg.is_some()).unwrap();
                    if c.bg.is_some() || c.padding > 0.0 {
                        blocks.push(Block::Group { blocks: c.blocks, bg: c.bg, padding: c.padding, align: ctx.align, max_width: None });
                    } else {
                        blocks.extend(c.blocks);
                    }
                }
                _ => blocks.push(Block::Row(cells)),
            }
        }
        if blocks.is_empty() {
            return;
        }
        let max_width = css.max_width.or(css.width_px).or_else(|| e.attr("width").and_then(px));
        let align = match e.attr("align").and_then(align) {
            Some(a) => a,
            None if css.auto_margin => Align::Center,
            None => ctx.align,
        };
        b.block(Block::Group { blocks, bg, padding: css.padding.unwrap_or(0.0), align, max_width });
    }
}

/// Unwraps groups that add nothing, so layout tables don't nest dozens deep.
fn simplify(blocks: Vec<Block>) -> Vec<Block> {
    let mut out = Vec::new();
    for b in blocks {
        match b {
            // A bare table of rows stays a group so its rows can sit flush.
            Block::Group { blocks, bg: None, padding, max_width: None, .. }
                if padding == 0.0 && !blocks.iter().all(|b| matches!(b, Block::Row(_))) =>
            {
                out.extend(simplify(blocks));
            }
            Block::Group { blocks, bg, padding, align, max_width } => {
                let blocks = simplify(blocks);
                if !blocks.is_empty() || bg.is_some() {
                    out.push(Block::Group { blocks, bg, padding, align, max_width });
                }
            }
            Block::Quote(bs) => out.push(Block::Quote(simplify(bs))),
            Block::List { ordered, items } => {
                out.push(Block::List { ordered, items: items.into_iter().map(simplify).collect() })
            }
            Block::Row(cells) => out.push(Block::Row(
                cells.into_iter().map(|c| Cell { blocks: simplify(c.blocks), ..c }).collect(),
            )),
            b => out.push(b),
        }
    }
    out
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "aside" | "nav" | "address"
            | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "dl" | "dt" | "dd" | "figure" | "figcaption"
            | "center" | "body" | "html" | "form" | "fieldset" | "tr" | "td" | "th" | "caption"
    )
}

fn heading(name: &str) -> Option<f32> {
    Some(match name {
        "h1" => 24.0,
        "h2" => 20.0,
        "h3" => 17.0,
        "h4" => 15.0,
        "h5" | "h6" => BASE_SIZE,
        _ => return None,
    })
}

pub fn openable(href: &str) -> bool {
    let h = href.to_ascii_lowercase();
    h.starts_with("http://") || h.starts_with("https://") || h.starts_with("mailto:")
}

/// The CSS properties this renderer reads from a `style` attribute.
#[derive(Default)]
struct Css {
    display_none: bool,
    invisible: bool,
    zero: bool,
    color: Option<Rgba>,
    background: Option<Rgba>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    mono: bool,
    size: Option<f32>,
    align: Option<Align>,
    padding: Option<f32>,
    width_px: Option<f32>,
    width_pct: Option<f32>,
    height_px: Option<f32>,
    max_width: Option<f32>,
    auto_margin: bool,
    /// `font-size: 0`.
    tiny: bool,
}

impl Css {
    fn parse(style: &str) -> Css {
        let mut c = Css::default();
        for decl in style.split(';') {
            let Some((k, v)) = decl.split_once(':') else { continue };
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().trim_end_matches("!important").trim().to_ascii_lowercase();
            match k.as_str() {
                "display" => c.display_none = v == "none",
                "visibility" => c.invisible = v == "hidden",
                "opacity" => c.zero |= v.parse::<f32>().is_ok_and(|o| o == 0.0),
                "max-height" if v.starts_with('0') && px(&v) == Some(0.0) => c.zero = true,
                "font-size" if v.starts_with('0') && px(&v) == Some(0.0) => c.tiny = true,
                "mso-hide" => c.display_none |= v == "all",
                "color" => c.color = color(&v),
                "background-color" => c.background = color(&v),
                "background" => c.background = v.split_whitespace().find_map(color),
                "font-weight" => {
                    c.bold = Some(v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|w| w >= 600))
                }
                "font-style" => c.italic = Some(v == "italic" || v == "oblique"),
                "text-decoration" | "text-decoration-line" => c.underline = Some(v.contains("underline")),
                "font-family" => c.mono = v.contains("mono") || v.contains("courier") || v.contains("consolas"),
                "font-size" => c.size = font_size(&v),
                "text-align" => c.align = align(&v),
                "padding" => c.padding = v.split_whitespace().next().and_then(px),
                "width" => {
                    c.width_px = px(&v);
                    c.width_pct = percent(&v);
                }
                "height" => c.height_px = px(&v),
                "max-width" => c.max_width = px(&v),
                "margin" | "margin-left" | "margin-right" => c.auto_margin |= v.contains("auto"),
                _ => {}
            }
        }
        c
    }

    fn hidden(&self) -> bool {
        self.display_none || self.invisible || self.zero
    }

    fn apply(&self, ctx: &mut Ctx, cx: &mut Cx) {
        if self.tiny {
            ctx.silent = true;
        } else if self.size.is_some() {
            ctx.silent = false;
        }
        let st = &mut ctx.style;
        if let Some(c) = self.color {
            st.color = Some(c);
            cx.colored = true;
        }
        if let Some(b) = self.bold {
            st.bold = b;
        }
        if let Some(i) = self.italic {
            st.italic = i;
        }
        if let Some(u) = self.underline {
            st.underline = u;
        }
        st.mono |= self.mono;
        if let Some(s) = self.size {
            st.size = Some(s);
        }
        if let Some(a) = self.align {
            ctx.align = a;
        }
    }
}

fn px(v: &str) -> Option<f32> {
    let v = v.trim();
    let n = v.strip_suffix("px").unwrap_or(v).trim();
    n.parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}

fn percent(v: &str) -> Option<f32> {
    v.trim().strip_suffix('%')?.trim().parse::<f32>().ok().filter(|p| *p > 0.0 && *p <= 100.0)
}

fn font_size(v: &str) -> Option<f32> {
    let size = if let Some(n) = v.strip_suffix("px") {
        n.trim().parse::<f32>().ok()? * 0.9
    } else if let Some(n) = v.strip_suffix("pt") {
        n.trim().parse::<f32>().ok()? * 1.2
    } else if let Some(n) = v.strip_suffix("em").or_else(|| v.strip_suffix("rem")) {
        n.trim().parse::<f32>().ok()? * BASE_SIZE
    } else {
        match v {
            "small" => 11.5,
            "medium" => BASE_SIZE,
            "large" => 16.0,
            "x-large" => 20.0,
            _ => return None,
        }
    };
    size.is_finite().then(|| size.clamp(9.0, 36.0))
}

fn align(v: &str) -> Option<Align> {
    match v.trim().to_ascii_lowercase().as_str() {
        "left" | "start" | "justify" => Some(Align::Start),
        "center" | "middle" => Some(Align::Center),
        "right" | "end" => Some(Align::End),
        _ => None,
    }
}

/// `#rgb`, `#rrggbb`, `rgb()`, `rgba()` and the common names, as sRGB bytes.
pub fn color(v: &str) -> Option<Rgba> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        // Lengths below are in bytes; anything but hex digits isn't a colour anyway.
        if !h.is_ascii() {
            return None;
        }
        let n = |s: &str| u8::from_str_radix(s, 16).ok();
        return match h.len() {
            3 => {
                let d = |i: usize| n(&h[i..i + 1]).map(|x| x * 17);
                Some([d(0)?, d(1)?, d(2)?, 255])
            }
            6 => Some([n(&h[0..2])?, n(&h[2..4])?, n(&h[4..6])?, 255]),
            8 => Some([n(&h[0..2])?, n(&h[2..4])?, n(&h[4..6])?, n(&h[6..8])?]),
            _ => None,
        };
    }
    if let Some(args) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let parts: Vec<f32> =
            args.trim_end_matches(')').split([',', ' ', '/']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        let a = parts.get(3).map_or(255, |a| (a.clamp(0.0, 1.0) * 255.0) as u8);
        return Some([*parts.first()? as u8, *parts.get(1)? as u8, *parts.get(2)? as u8, a]);
    }
    let named = match v.as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "navy" => [0, 0, 128],
        "maroon" => [128, 0, 0],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "olive" => [128, 128, 0],
        "orange" => [255, 165, 0],
        "yellow" => [255, 255, 0],
        _ => return None,
    };
    Some([named[0], named[1], named[2], 255])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> Doc {
        parse(html, &|id| Some(format!("blob:{id}")))
    }

    #[test]
    fn paragraphs_collapse_whitespace_and_keep_styled_runs() {
        let d = doc("<p>Hello   <b>bold</b>\n world</p><p>Two<br>lines</p>");
        let Block::Text { text, runs, .. } = &d.blocks[0] else { panic!("{:?}", d.blocks) };
        assert_eq!(text, "Hello bold world");
        assert!(runs.iter().any(|r| r.style.bold && &text[r.start..r.end] == "bold"));
        assert!(matches!(&d.blocks[1], Block::Text { text, .. } if text == "Two\nlines"));
    }

    #[test]
    fn links_images_and_hidden_preheaders() {
        let d = doc(r#"<div style="display:none">preheader</div>
            <a style="font-weight:normal" href="https://example.com">site</a><img src="cid:logo" width="120">
            <img src="https://t.example/p.gif" width="1" height="1">"#);
        let Block::Text { text, runs, .. } = &d.blocks[0] else { panic!("{:?}", d.blocks) };
        assert_eq!(text, "site");
        assert_eq!(runs[0].style.link.as_deref(), Some("https://example.com"));
        assert_eq!(runs[0].style.color, None, "a link drops the inherited text colour");
        assert!(matches!(&d.blocks[1], Block::Image { src, width: Some(120.0), .. } if src == "blob:logo"));
        assert_eq!(d.blocks.len(), 2, "the tracking pixel and preheader are dropped");
        assert_eq!(d.trackers, ["Tracking pixel"]);
        assert!(!d.remote_images);
    }

    #[test]
    fn zero_font_size_hides_only_its_own_text() {
        // MJML's layout: font-size 0 on the cell closes the gaps between columns; the content
        // inside sets its size again. A preheader at size 0 stays hidden.
        let d = doc(r#"<div style="font-size:0px;max-height:none">preheader</div>
            <table><tr><td style="direction:ltr;font-size:0px;padding:20px 0">
              <div style="font-size:0px;display:inline-block;width:100%"><table><tr>
                <td style="font-size:0px;padding:10px 25px"><div style="font-family:Arial;font-size:16px;color:#141413">Your sign-in link</div></td>
              </tr></table></div>
            </td></tr></table>"#);
        let texts: Vec<String> = all_text(&d.blocks);
        assert_eq!(texts, ["Your sign-in link"]);
    }

    fn all_text(blocks: &[Block]) -> Vec<String> {
        blocks.iter().flat_map(|b| match b {
            Block::Text { text, .. } => vec![text.clone()],
            Block::Group { blocks, .. } | Block::Quote(blocks) => all_text(blocks),
            Block::Row(cells) => cells.iter().flat_map(|c| all_text(&c.blocks)).collect(),
            _ => Vec::new(),
        }).collect()
    }

    #[test]
    fn layout_tables_flatten_and_columns_become_rows() {
        let d = doc(r##"<table width="600" align="center" bgcolor="#ffffff"><tr><td>
            <table><tr><td width="50%">Left</td><td width="50%">Right</td></tr></table>
            </td></tr></table>"##);
        let Block::Group { blocks, max_width: Some(600.0), align: Align::Center, bg: Some(_), .. } = &d.blocks[0] else {
            panic!("{:?}", d.blocks)
        };
        let Block::Group { blocks: rows, .. } = &blocks[0] else { panic!("the inner table stays a group: {blocks:?}") };
        let Block::Row(cells) = &rows[0] else { panic!("{rows:?}") };
        assert_eq!(cells.len(), 2);
        assert!(matches!(&cells[0].blocks[0], Block::Text { align: Align::Start, .. }), "a centred table keeps its text's alignment");
        assert_eq!(cells[0].width, Width::Percent(50.0));
        assert!(d.colored);
    }

    #[test]
    fn odd_sizes_and_colours_dont_panic() {
        assert_eq!(color("#a\u{e9}"), None);
        assert_eq!(color("#a\u{e9}\u{e9}a"), None);
        let d = doc("<p style='color:#a\u{e9}'>one</p><font size='NaN' bgcolor='#a\u{e9}\u{e9}a'>two</font>");
        assert_eq!(all_text(&d.blocks), ["one", "two"]);
        assert_eq!(font_size("NaNpx"), None);
    }

    #[test]
    fn deeply_nested_mail_shows_its_text_quickly() {
        let html = format!("{}deep text{}", "<div>".repeat(100_000), "</div>".repeat(100_000));
        let start = std::time::Instant::now();
        let d = doc(&html);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(all_text(&d.blocks), ["deep text"]);
        assert!(!too_deep(&"<table><tr><td><p>x".repeat(300)));
    }

    #[test]
    fn colours_parse() {
        assert_eq!(color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(color("rgb(10, 20, 30)"), Some([10, 20, 30, 255]));
        assert_eq!(color("Navy"), Some([0, 0, 128, 255]));
        assert_eq!(color("inherit"), None);
    }
}
