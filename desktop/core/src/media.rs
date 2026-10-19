//! What a row shows under its words (macOS `ChatStrip`): its voice note, a tile for each file it
//! names, and a card for each stored preview of a page; and its words less every line that only
//! names what the strip shows (`words`). It is read off the row alone and never looks at the disk: whether
//! a file is there is each tile's to find out when it reads it. Also the two bounds on reading
//! pictures: the most decoded at once, and the largest decoded at all.

use crate::chat_wire::protocol::{is_media_ref, Block, LinkPreview, MediaRef, Row};
use crate::places::{opened_by, WebAddress};
use std::collections::{HashSet, VecDeque};

/// The most link cards a row draws (the contract's bound on stored previews).
pub const MAX_LINKS: usize = 4;
/// The most pictures decoded at once, and the most waiting their turn (Mac `ChatPreviewBudget`).
pub const MAX_DECODING: usize = 2;
pub const MAX_WAITING: usize = 64;
/// The largest picture decoded, in pixels: past it, a picture shows as its icon.
pub const PICTURE_PIXELS: u64 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileKind {
    /// Drawn from its file in the strip, and opened in the app's viewer.
    Picture,
    /// Drawn as its icon, name and size, and opened in the computer's own app.
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub media: MediaRef,
    pub kind: TileKind,
    /// The file's name, or what it is where the row names none.
    pub name: String,
    /// What it is and how big: `Picture · 182 kB`.
    pub detail: String,
    /// What a screen reader hears: `Picture, lamp.jpg`.
    pub spoken: String,
}

/// A stored preview of a page the row links, as its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCard {
    /// The web address a click opens: the one the text wrote, else the one the page answered
    /// from.
    pub opens: String,
    pub title: String,
    pub site: String,
    /// The site's first letter or digit, for the card's picture until its own arrives.
    pub initial: String,
    /// The page's picture, a ref the daemon grants like any file.
    pub image_ref: Option<String>,
    pub spoken: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Strip {
    /// The row's voice note: its first Ogg sound, which a phone records.
    pub voice: Option<MediaRef>,
    /// Every other file, once each, pictures first.
    pub tiles: Vec<Tile>,
    pub links: Vec<LinkCard>,
}

impl Strip {
    /// Whether nothing is drawn under the words.
    pub fn is_empty(&self) -> bool {
        self.voice.is_none() && self.tiles.is_empty() && self.links.is_empty()
    }
}

pub fn strip(row: &Row) -> Strip {
    let voice = row.media_refs.iter().find(|media| is_voice_note(media));
    let mut seen: HashSet<&str> = voice.iter().map(|note| note.r#ref.as_str()).collect();
    let (mut tiles, files): (Vec<Tile>, Vec<Tile>) = row
        .media_refs
        .iter()
        .filter(|media| seen.insert(media.r#ref.as_str()))
        .map(tile)
        .partition(|tile| tile.kind == TileKind::Picture);
    tiles.extend(files);
    Strip {
        voice: voice.cloned(),
        tiles,
        links: link_cards(&row.link_previews, &row.blocks),
    }
}

/// The row's words as drawn under `strip`, its strip: less every line that only names a tile or
/// a card. Empty where nothing else was written, and the row then draws its strip alone.
pub fn words(row: &Row, strip: &Strip) -> String {
    let shown = shown_names(strip, &row.link_previews);
    without_shown_lines(&row.text, &shown)
}

/// A phone's voice note: Ogg, the only sound this app plays in a row (the engine names every
/// `OggS` file `audio/ogg`).
fn is_voice_note(media: &MediaRef) -> bool {
    media.kind == "audio" && media.mime.eq_ignore_ascii_case("audio/ogg")
}

pub fn tile(media: &MediaRef) -> Tile {
    let word = kind_word(media);
    let mime = media.mime.to_ascii_lowercase();
    // An SVG is drawn as its icon, as on the Mac: daemon content is never rendered as a vector.
    let picture = mime.starts_with("image/") && mime != "image/svg+xml";
    let named = media
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    Tile {
        media: media.clone(),
        kind: if picture {
            TileKind::Picture
        } else {
            TileKind::File
        },
        name: named.unwrap_or(word).to_owned(),
        // A tile named by its kind says it once.
        detail: match named {
            Some(_) => format!("{word} · {}", format_size(media.size_bytes)),
            None => format_size(media.size_bytes),
        },
        spoken: named.map_or(word.to_owned(), |name| format!("{word}, {name}")),
    }
}

/// The name Save As starts from: a bare name, never a path the phone wrote. The last segment of
/// the tile's name without control characters, or the kind word where nothing but dots is left.
pub fn save_name(tile: &Tile) -> String {
    let last = tile
        .name
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let bare: String = last.chars().filter(|c| !c.is_control()).collect();
    let bare = bare.trim();
    if bare.chars().all(|c| c == '.') {
        return kind_word(&tile.media).to_owned();
    }
    bare.to_owned()
}

/// What a file is, in the macOS app's words (`chat.kind.*`, `chat.voiceNote`).
pub fn kind_word(media: &MediaRef) -> &'static str {
    match (
        media.kind.as_str(),
        media.mime.to_ascii_lowercase().as_str(),
    ) {
        ("audio", "audio/ogg") => "Voice note",
        ("image", _) => "Picture",
        ("video", _) => "Video",
        ("audio", _) => "Sound",
        (_, "application/pdf") => "PDF",
        _ => "File",
    }
}

/// A card for each stored preview, in order: only a web address, once, and none for a page a
/// place under the reply stands for, whose card opens it.
pub fn link_cards(previews: &[LinkPreview], blocks: &[Block]) -> Vec<LinkCard> {
    let mut seen = HashSet::new();
    previews
        .iter()
        .filter_map(|preview| {
            let opens = preview.link.as_deref().unwrap_or(&preview.url);
            let address = WebAddress::parse(opens)?;
            let fresh = seen.insert(address.destination());
            (fresh && !opened_by(opens, blocks)).then(|| link_card(preview, opens, &address))
        })
        .take(MAX_LINKS)
        .collect()
}

fn link_card(preview: &LinkPreview, opens: &str, address: &WebAddress) -> LinkCard {
    let host = address.host();
    let title = match preview.title.trim() {
        "" => host.clone(),
        title => title.to_owned(),
    };
    let site = match preview.site.trim() {
        "" => host.strip_prefix("www.").unwrap_or(&host).to_owned(),
        site => site.to_owned(),
    };
    let initial = site
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    LinkCard {
        opens: opens.to_owned(),
        spoken: format!("{title}, Link"),
        title,
        site,
        initial,
        image_ref: preview.image_ref.clone().filter(|r| is_media_ref(r)),
    }
}

/// What the strip shows, as the text may name it: each tile's file name, and each card's address
/// as the text wrote it and as the page answered from.
fn shown_names(strip: &Strip, previews: &[LinkPreview]) -> Vec<String> {
    let carded: HashSet<String> = strip
        .links
        .iter()
        .map(|card| destination(&card.opens))
        .collect();
    // Trimmed as a tile's name is.
    let mut shown: Vec<String> = strip
        .tiles
        .iter()
        .filter_map(|tile| tile.media.filename.as_deref())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    for preview in previews {
        let opens = preview.link.as_deref().unwrap_or(&preview.url);
        if carded.contains(&destination(opens)) {
            shown.push(preview.url.clone());
            shown.extend(preview.link.clone());
        }
    }
    shown
}

/// A web address as two are compared; anything else as written.
fn destination(name: &str) -> String {
    WebAddress::parse(name).map_or_else(|| name.to_owned(), |address| address.destination())
}

/// The text less every line that names, and only names, something in `shown` (Mac
/// `withoutShownLines`): a line whose whole content, once its indent, a list marker, a code span
/// or angle brackets round it and the marks that close a sentence are set aside, is one shown
/// name; or a markdown link whose words are its target or that target's last segment, to a
/// shown name. Nothing inside a code fence is touched. Where a line goes, a blank line beside it
/// closes up, and the text loses its blank lines at both ends. A text that loses no line comes
/// back as it was.
pub fn without_shown_lines(text: &str, shown: &[String]) -> String {
    let shown: HashSet<String> = shown.iter().map(|name| destination(name)).collect();
    let mut kept: Vec<&str> = Vec::new();
    let (mut dropped, mut dropped_since_kept) = (false, false);
    let mut fence: Option<Fence> = None;
    for line in text.split('\n') {
        let code = in_code(&mut fence, line);
        if !code && names(line).is_some_and(|named| shown.contains(&destination(named))) {
            dropped = true;
            dropped_since_kept = true;
            continue;
        }
        if dropped_since_kept && is_blank(line) && kept.last().is_some_and(|last| is_blank(last)) {
            continue;
        }
        kept.push(line);
        dropped_since_kept = false;
    }
    if !dropped {
        return text.to_owned();
    }
    let first = kept
        .iter()
        .position(|line| !is_blank(line))
        .unwrap_or(kept.len());
    let last = kept
        .iter()
        .rposition(|line| !is_blank(line))
        .map_or(first, |at| at + 1);
    kept[first..last].join("\n")
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The one name a line says and nothing more, or `None`.
fn names(line: &str) -> Option<&str> {
    let body = without_list_marker(line.trim());
    let closed = body.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    if closed.len() > 2 && closed.starts_with('`') && closed.ends_with('`') {
        return wholly(&closed[1..closed.len() - 1]);
    }
    if let Some((words, target)) = markdown_link(closed) {
        return link_target(words, target);
    }
    let opening: &[char] = &['(', '<', '"', '\'', '\u{201C}', '\u{2018}'];
    let closing: &[char] = &[
        '.', ',', ';', ':', '!', '?', '\'', '"', '\u{201D}', '\u{2019}', '`', '>', ')', ']', '}',
    ];
    let opened = body.trim_start_matches(opening);
    wholly(with_own_brackets(
        opened,
        opened.trim_end_matches(closing).len(),
    ))
}

/// `text` up to `end`, and any `)` after it that closes a `(` inside it, as a link reader keeps
/// an address's own bracket (`https://en.test/Foo_(bar)`, Mac `isReading`).
fn with_own_brackets(text: &str, end: usize) -> &str {
    let open = text[..end].matches('(').count();
    let closed = text[..end].matches(')').count();
    let after = &text[end..];
    let owned = after.len() - after.trim_start_matches(')').len();
    &text[..end + owned.min(open.saturating_sub(closed))]
}

/// `[words](target)`, the whole of `text`.
fn markdown_link(text: &str) -> Option<(&str, &str)> {
    let inner = text.strip_prefix('[')?.strip_suffix(')')?;
    let (words, target) = inner.rsplit_once("](")?;
    (!words.is_empty() && !target.is_empty()).then_some((words, target))
}

/// A link's target, where its words are that target or the target's last segment.
fn link_target<'a>(words: &str, target: &'a str) -> Option<&'a str> {
    let words: String = words.chars().filter(|c| *c != '`').collect();
    let target = target
        .strip_prefix('<')
        .and_then(|inner| inner.strip_suffix('>'))
        .unwrap_or(target);
    let last = target
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if words != target && words != last {
        return None;
    }
    wholly(target)
}

/// Text that is one name from end to end: no space and no backtick in it.
fn wholly(text: &str) -> Option<&str> {
    let one = !text.is_empty() && !text.contains(|c: char| c.is_whitespace() || c == '`');
    one.then_some(text)
}

/// A list item's text, its marker gone: `-`, `*`, `+` or `•`, or a number and a full stop or
/// bracket, then a space.
fn without_list_marker(line: &str) -> &str {
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let rest = match line.strip_prefix(['-', '*', '+', '•']) {
        Some(rest) => rest,
        None if digits > 0 => match line[digits..].strip_prefix(['.', ')']) {
            Some(rest) => rest,
            None => return line,
        },
        None => return line,
    };
    let body = rest.trim_start();
    if body.len() == rest.len() {
        return line;
    }
    body
}

/// An open code fence: its mark and how many (Mac `ReplyFences`).
struct Fence {
    mark: char,
    length: usize,
}

/// Whether `line` belongs to a code block, its fences included, moving `fence` on.
fn in_code(fence: &mut Option<Fence>, line: &str) -> bool {
    let marks = line.trim_start_matches([' ', '\t']);
    if let Some(open) = fence {
        let run = marks.len() - marks.trim_start_matches(open.mark).len();
        if run >= open.length && marks[run..].trim().is_empty() {
            *fence = None;
        }
        return true;
    }
    let Some(mark) = marks
        .chars()
        .next()
        .filter(|mark| matches!(mark, '`' | '~'))
    else {
        return false;
    };
    let run = marks.len() - marks.trim_start_matches(mark).len();
    let opens = run >= 3 && (mark == '~' || !marks[run..].contains('`'));
    if opens {
        *fence = Some(Fence { mark, length: run });
    }
    opens
}

/// A size as the desktop writes one: decimal units, three figures, `48.2 kB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    match bytes {
        1 => return "1 byte".to_owned(),
        0..1000 => return format!("{bytes} bytes"),
        _ => {}
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.5 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    let decimals = match value {
        v if v < 9.995 => 2,
        v if v < 99.95 => 1,
        _ => 0,
    };
    let written = format!("{value:.decimals$}");
    let figures = if written.contains('.') {
        written.trim_end_matches('0').trim_end_matches('.')
    } else {
        &written
    };
    format!("{figures} {}", UNITS[unit])
}

/// Whether a picture of this size, as its header says, may be decoded: both sides positive and
/// the area within `PICTURE_PIXELS`, worked out in 64 bits so no two sides overflow it.
pub fn within_ceiling(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && u64::from(width) * u64::from(height) <= PICTURE_PIXELS
}

/// How a picture meets the square it is decoded for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Its shorter side fills the square: a tile, cropped to its shape.
    Cover,
    /// Its longer side fits the square: the viewer, whole.
    Contain,
}

/// The size to scale a `width` by `height` picture to for a square of `side` pixels, never
/// larger than the picture and never below a pixel. A square, so one number serves a picture
/// standing either way; the drawn shape then crops it (a tile) or leaves a margin (the viewer).
pub fn decode_size(width: u32, height: u32, side: u32, fit: Fit) -> (u32, u32) {
    debug_assert!(
        width > 0 && height > 0 && side > 0,
        "decode_size of {width}x{height} for {side}"
    );
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let edge = match fit {
        Fit::Cover => w.min(h),
        Fit::Contain => w.max(h),
    };
    let scale = (f64::from(side.max(1)) / edge).min(1.0);
    // Within u32: each side is at most the picture's own.
    let scaled = |length: f64| ((length * scale).round() as u32).max(1);
    (scaled(w), scaled(h))
}

/// Which decodes run: at most `MAX_DECODING` at once, and `MAX_WAITING` waiting, the oldest
/// waiting giving way to a newer one.
#[derive(Debug)]
pub struct Admission<T> {
    running: usize,
    waiting: VecDeque<T>,
}

impl<T> Default for Admission<T> {
    fn default() -> Self {
        Admission {
            running: 0,
            waiting: VecDeque::new(),
        }
    }
}

impl<T> Admission<T> {
    /// The job to start now, if it may, and the waiting job it pushed out, if any.
    pub fn ask(&mut self, job: T) -> (Option<T>, Option<T>) {
        if self.running < MAX_DECODING {
            self.running += 1;
            return (Some(job), None);
        }
        let dropped = if self.waiting.len() >= MAX_WAITING {
            self.waiting.pop_front()
        } else {
            None
        };
        self.waiting.push_back(job);
        (None, dropped)
    }

    /// A running job ended: the next to start, if one waits.
    pub fn done(&mut self) -> Option<T> {
        debug_assert!(self.running > 0, "a job ended that never started");
        let next = self.waiting.pop_front();
        if next.is_none() {
            self.running = self.running.saturating_sub(1);
        }
        next
    }
}
