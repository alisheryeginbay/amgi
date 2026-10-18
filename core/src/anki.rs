//! Reading Anki packages (`.apkg`, `.colpkg`) and converting them to Markdown decks.
//!
//! A package is a zip archive holding the collection as SQLite: `collection.anki21b`
//! (zstd-compressed, schema 18, current Anki) or `collection.anki21` / `collection.anki2`
//! (legacy schema 11, where note types and decks are JSON in the `col` table).
//! Images and sounds used by cards are copied to a `media` folder.

mod text;

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::parser;
pub use text::Media;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an Anki package: {0}")]
    Format(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

type Result<T, E = Error> = std::result::Result<T, E>;

struct Notetype {
    name: String,
    cloze: bool,
    fields: Vec<String>,
    /// Template name, question format, answer format; indexed by card ordinal.
    templates: Vec<(String, String, String)>,
}

struct Note {
    id: i64,
    notetype: i64,
    fields: Vec<String>,
    tags: String,
}

struct AnkiCard {
    id: i64,
    note: i64,
    deck: i64,
    ord: u32,
    is_new: bool,
    suspended: bool,
}

/// One entry of Anki's review log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Revlog {
    /// Unix seconds.
    pub time: i64,
    /// 1–4 for Again–Easy; 0 for manual changes.
    pub ease: u8,
    /// 0 learn, 1 review, 2 relearn, 3 filtered, 4 manual (reschedule or reset).
    pub kind: u8,
    /// Days if positive, seconds if negative.
    pub interval: i64,
    /// Ease in permille, or difficulty under FSRS. 0 on a filtered-deck review with
    /// rescheduling off, which left the card's schedule alone.
    pub factor: i64,
}

impl Revlog {
    pub fn interval_secs(&self) -> i64 {
        if self.interval < 0 { -self.interval } else { self.interval * 86_400 }
    }
}

pub struct Package {
    /// The package file, read again to copy media.
    source: Option<PathBuf>,
    /// Media file name → zip entry holding it.
    media: HashMap<String, String>,
    /// Deck id → name components (`Languages::Kazakh` → `["Languages", "Kazakh"]`).
    decks: HashMap<i64, Vec<String>>,
    notetypes: HashMap<i64, Notetype>,
    notes: Vec<Note>,
    cards: Vec<AnkiCard>,
    revlog: HashMap<i64, Vec<Revlog>>,
}

impl Package {
    pub fn read(path: &Path) -> Result<Self> {
        let mut zip = zip::ZipArchive::new(File::open(path)?)?;
        // Newer packages also hold a stub `collection.anki2` asking to update Anki.
        let name = ["collection.anki21b", "collection.anki21", "collection.anki2"]
            .into_iter()
            .find(|name| zip.index_for_name(name).is_some())
            .ok_or_else(|| Error::Format("no collection inside".into()))?;
        let bytes = read_entry(&mut zip, name)?;
        // SQLite needs a file; read everything up front and remove it.
        let temp = TempFile::new(&bytes)?;
        let conn = Connection::open_with_flags(&temp.0, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut package = Self::from_db(&conn)?;
        package.source = Some(path.to_path_buf());
        if zip.index_for_name("media").is_some() {
            package.media = media_index(&read_entry(&mut zip, "media")?)?;
        }
        Ok(package)
    }

    /// Copy the named media files into `dir`, keeping files already there.
    /// Returns how many of them are in `dir` afterwards.
    pub fn write_media(&self, names: &BTreeSet<String>, dir: &Path) -> Result<u32> {
        let Some(source) = &self.source else { return Ok(0) };
        let mut zip = zip::ZipArchive::new(File::open(source)?)?;
        let mut present = 0;
        for name in names {
            let target = dir.join(name);
            if !target.exists() {
                let Some(entry) = self.media.get(name) else { continue };
                let bytes = read_entry(&mut zip, entry)?;
                fs::create_dir_all(dir)?;
                fs::write(&target, bytes)?;
            }
            present += 1;
        }
        Ok(present)
    }

    fn from_db(conn: &Connection) -> Result<Self> {
        // Anki declares names with its own case-insensitive collation; SQLite refuses
        // to query those tables until a collation by that name exists.
        conn.create_collation("unicase", |a, b| a.to_lowercase().cmp(&b.to_lowercase()))?;
        let modern: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'notetypes'",
            [],
            |r| r.get(0),
        )?;
        let (decks, notetypes) = if modern { read_schema18(conn)? } else { read_schema11(conn)? };

        let notes = conn
            .prepare("SELECT id, mid, flds, tags FROM notes ORDER BY id")?
            .query_map([], |r| {
                Ok(Note {
                    id: r.get(0)?,
                    notetype: r.get(1)?,
                    fields: r.get::<_, String>(2)?.split('\x1f').map(String::from).collect(),
                    tags: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        // Cards in a filtered deck remember their home deck in `odid`.
        let cards = conn
            .prepare(
                "SELECT id, nid, CASE WHEN odid != 0 THEN odid ELSE did END, ord, type, queue
                 FROM cards ORDER BY nid, ord",
            )?
            .query_map([], |r| {
                Ok(AnkiCard {
                    id: r.get(0)?,
                    note: r.get(1)?,
                    deck: r.get(2)?,
                    ord: r.get(3)?,
                    is_new: r.get::<_, i64>(4)? == 0,
                    suspended: r.get::<_, i64>(5)? == -1,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut revlog: HashMap<i64, Vec<Revlog>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT cid, id, ease, type, ivl, factor FROM revlog ORDER BY id")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            revlog.entry(r.get(0)?).or_default().push(Revlog {
                time: r.get::<_, i64>(1)? / 1000,
                ease: r.get(2)?,
                kind: r.get(3)?,
                interval: r.get(4)?,
                factor: r.get(5)?,
            });
        }
        Ok(Package {
            source: None,
            media: HashMap::new(),
            decks,
            notetypes,
            notes,
            cards,
            revlog,
        })
    }
}

/// A zip entry's contents, decompressed if the package stored it with zstd.
fn read_entry(zip: &mut zip::ZipArchive<File>, name: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    zip.by_name(name)?.read_to_end(&mut bytes)?;
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        bytes = zstd::decode_all(bytes.as_slice())?;
    }
    Ok(bytes)
}

/// Media file name → zip entry, from the package's `media` file: JSON
/// `{"0": "name.png"}` in legacy packages, otherwise a protobuf `MediaEntries`
/// whose entries are stored by index.
fn media_index(bytes: &[u8]) -> Result<HashMap<String, String>> {
    if bytes.first() == Some(&b'{') {
        let map: HashMap<String, String> = serde_json::from_slice(bytes)?;
        return Ok(map.into_iter().map(|(entry, name)| (name, entry)).collect());
    }
    let mut index = HashMap::new();
    // MediaEntries.entries = 1; MediaEntry.name = 1, legacy_zip_filename = 255.
    for (i, (_, entry)) in proto_fields(bytes).filter(|(n, _)| *n == 1).enumerate() {
        let Proto::Bytes(entry) = entry else { continue };
        let (mut name, mut zip_name) = (None, i.to_string());
        for field in proto_fields(entry) {
            match field {
                (1, Proto::Bytes(b)) => name = Some(String::from_utf8_lossy(b).into_owned()),
                (255, Proto::Varint(n)) => zip_name = n.to_string(),
                _ => {}
            }
        }
        if let Some(name) = name {
            index.insert(name, zip_name);
        }
    }
    Ok(index)
}

type Decks = HashMap<i64, Vec<String>>;
type Notetypes = HashMap<i64, Notetype>;

/// Legacy collections keep decks and note types as JSON in the `col` table.
fn read_schema11(conn: &Connection) -> Result<(Decks, Notetypes)> {
    let (decks_json, models_json): (String, String) =
        conn.query_row("SELECT decks, models FROM col", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let decks_json: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&decks_json)?;
    let models_json: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&models_json)?;

    let mut decks = HashMap::new();
    for (id, deck) in decks_json {
        if let (Ok(id), Some(name)) = (id.parse(), deck["name"].as_str()) {
            decks.insert(id, name.split("::").map(String::from).collect());
        }
    }
    let str_of = |v: &serde_json::Value, key: &str| v[key].as_str().unwrap_or_default().to_string();
    let mut notetypes = HashMap::new();
    for (id, model) in models_json {
        let Ok(id) = id.parse() else { continue };
        let mut fields: Vec<_> = model["flds"].as_array().into_iter().flatten().collect();
        fields.sort_by_key(|f| f["ord"].as_i64());
        let mut templates: Vec<_> = model["tmpls"].as_array().into_iter().flatten().collect();
        templates.sort_by_key(|t| t["ord"].as_i64());
        notetypes.insert(
            id,
            Notetype {
                name: str_of(&model, "name"),
                cloze: model["type"].as_i64() == Some(1),
                fields: fields.iter().map(|f| str_of(f, "name")).collect(),
                templates: templates
                    .iter()
                    .map(|t| (str_of(t, "name"), str_of(t, "qfmt"), str_of(t, "afmt")))
                    .collect(),
            },
        );
    }
    Ok((decks, notetypes))
}

/// Current collections use tables, with configuration stored as protobuf.
fn read_schema18(conn: &Connection) -> Result<(Decks, Notetypes)> {
    let mut decks = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, name FROM decks")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let name: String = r.get(1)?;
        decks.insert(r.get(0)?, name.split('\x1f').map(String::from).collect());
    }

    let mut notetypes = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, name, config FROM notetypes")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let config: Vec<u8> = r.get(2)?;
        // Notetype.Config: `kind = 1`, where 1 is cloze.
        let cloze = proto_fields(&config).any(|(n, v)| n == 1 && matches!(v, Proto::Varint(1)));
        let notetype = Notetype {
            name: r.get(1)?,
            cloze,
            fields: Vec::new(),
            templates: Vec::new(),
        };
        notetypes.insert(r.get::<_, i64>(0)?, notetype);
    }
    let mut stmt = conn.prepare("SELECT ntid, name FROM fields ORDER BY ntid, ord")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        if let Some(nt) = notetypes.get_mut(&r.get::<_, i64>(0)?) {
            nt.fields.push(r.get(1)?);
        }
    }
    let mut stmt = conn.prepare("SELECT ntid, name, config FROM templates ORDER BY ntid, ord")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let config: Vec<u8> = r.get(2)?;
        // Template.Config: `q_format = 1`, `a_format = 2`.
        let format = |field| {
            proto_fields(&config)
                .find_map(|(n, v)| match v {
                    Proto::Bytes(b) if n == field => Some(String::from_utf8_lossy(b).into_owned()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        if let Some(nt) = notetypes.get_mut(&r.get::<_, i64>(0)?) {
            nt.templates.push((r.get(1)?, format(1), format(2)));
        }
    }
    Ok((decks, notetypes))
}

enum Proto<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Other,
}

/// Top-level fields of a protobuf message. Stops at the first malformed field.
fn proto_fields(mut buf: &[u8]) -> impl Iterator<Item = (u32, Proto<'_>)> {
    fn varint(buf: &mut &[u8]) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&b, rest) = buf.split_first()?;
            *buf = rest;
            value |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    std::iter::from_fn(move || {
        let key = varint(&mut buf)?;
        let value = match key & 7 {
            0 => Proto::Varint(varint(&mut buf)?),
            1 => {
                buf = buf.get(8..)?;
                Proto::Other
            }
            2 => {
                let len = usize::try_from(varint(&mut buf)?).ok()?;
                let bytes = buf.get(..len)?;
                buf = &buf[len..];
                Proto::Bytes(bytes)
            }
            5 => {
                buf = buf.get(4..)?;
                Proto::Other
            }
            _ => return None,
        };
        Some(((key >> 3) as u32, value))
    })
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(bytes: &[u8]) -> std::io::Result<Self> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("amgi-import-{}-{n}.sqlite", std::process::id()));
        fs::write(&path, bytes)?;
        Ok(TempFile(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A converted package: Markdown per deck, plus the review history of each card.
#[derive(Debug, Default)]
pub struct Conversion {
    /// Deck path (`Languages/Kazakh`) and Markdown, in Anki deck order.
    pub decks: Vec<(String, String)>,
    /// amgi card id and the Anki review log of the card it came from. Cards that
    /// were new in Anki are absent.
    pub histories: Vec<(String, Vec<Revlog>)>,
    pub notes: u32,
    pub cards: u32,
    /// Notes that produced no card, such as image occlusion or empty templates.
    pub skipped_notes: u32,
    /// Suspended Anki cards, written with the `#suspended` tag.
    pub suspended_cards: u32,
    pub media: Media,
}

impl Package {
    pub fn convert(&self) -> Conversion {
        let mut out = Conversion::default();
        let mut decks: HashMap<i64, String> = HashMap::new();
        let mut cards_by_note: HashMap<i64, Vec<&AnkiCard>> = HashMap::new();
        for card in &self.cards {
            cards_by_note.entry(card.note).or_default().push(card);
        }

        for note in &self.notes {
            let (Some(notetype), Some(cards)) = (self.notetypes.get(&note.notetype), cards_by_note.get(&note.id))
            else {
                continue;
            };
            out.notes += 1;
            let active: Vec<&AnkiCard> = cards.iter().copied().filter(|c| !c.suspended).collect();
            let Some(first) = active.first().or(cards.first()) else { continue };

            let mut fields: HashMap<&str, &str> = notetype
                .fields
                .iter()
                .map(String::as_str)
                .zip(note.fields.iter().map(String::as_str))
                .collect();
            fields.insert("Tags", note.tags.trim());
            fields.insert("Type", &notetype.name);

            let before = out.cards;
            if notetype.cloze {
                out.media.prefix = self.media_prefix(first.deck);
                // Each Anki card is one cloze number, but amgi suspends a cloze block as a
                // whole. So suspended numbers get a block of their own, tagged, beside the
                // block of those still studied: the sentence is written twice.
                let suspended: Vec<&AnkiCard> = cards.iter().copied().filter(|c| c.suspended).collect();
                for (group, is_suspended) in [(&active, false), (&suspended, true)] {
                    if group.is_empty() {
                        continue;
                    }
                    let numbers: BTreeSet<u32> = group.iter().map(|c| c.ord + 1).collect();
                    let Some((block, ids)) = cloze_block(notetype, &fields, &numbers, &mut out.media) else { continue };
                    let block = with_tags(&block, &note_tags(&note.tags, is_suspended));
                    self.add_histories(group, |ord| ids_for_cloze(&ids, ord), &mut out);
                    out.cards += ids.len() as u32;
                    if is_suspended {
                        out.suspended_cards += ids.len() as u32;
                    }
                    push_block(&mut decks, first.deck, &block);
                }
            } else {
                for card in cards {
                    let Some((name, qfmt, afmt)) = notetype.templates.get(card.ord as usize) else { continue };
                    let deck = self.deck_name(card.deck);
                    let mut fields = fields.clone();
                    fields.insert("Card", name);
                    fields.insert("Deck", &deck);
                    out.media.prefix = self.media_prefix(card.deck);
                    let Some((block, id)) = basic_block(qfmt, afmt, &fields, &mut out.media) else { continue };
                    let block = with_tags(&block, &note_tags(&note.tags, card.suspended));
                    self.add_histories(&[card], |_| vec![id.clone()], &mut out);
                    out.cards += 1;
                    out.suspended_cards += u32::from(card.suspended);
                    push_block(&mut decks, card.deck, &block);
                }
            }
            if out.cards == before {
                out.skipped_notes += 1;
            }
        }

        let mut order: Vec<i64> = decks.keys().copied().collect();
        order.sort_by_key(|id| self.decks.get(id));
        out.decks = order
            .into_iter()
            .map(|id| {
                let path = self.deck_path(id);
                let mut markdown = decks.remove(&id).unwrap_or_default();
                // Keep the Anki name when the file name had to change.
                let name = self.decks.get(&id).and_then(|parts| parts.last());
                if let Some(name) = name.filter(|n| Some(n.as_str()) != path.rsplit('/').next()) {
                    markdown = format!("+++\nname = {}\n+++\n\n{markdown}", toml_string(name));
                }
                (path, markdown)
            })
            .collect();
        out
    }

    fn add_histories(&self, cards: &[&AnkiCard], ids: impl Fn(u32) -> Vec<String>, out: &mut Conversion) {
        for card in cards.iter().filter(|c| !c.is_new) {
            if let Some(log) = self.revlog.get(&card.id) {
                for id in ids(card.ord) {
                    out.histories.push((id, log.clone()));
                }
            }
        }
    }

    /// Path from a deck's file to the `media` folder at the collection root.
    fn media_prefix(&self, deck: i64) -> String {
        "../".repeat(self.deck_path(deck).matches('/').count()) + "media/"
    }

    fn deck_name(&self, id: i64) -> String {
        self.decks.get(&id).map(|parts| parts.join("::")).unwrap_or_default()
    }

    /// File path for a deck, safe for any file system and never hidden.
    fn deck_path(&self, id: i64) -> String {
        let parts = self.decks.get(&id).cloned().unwrap_or_else(|| vec!["Default".into()]);
        parts
            .iter()
            .map(|part| {
                let clean: String = part
                    .chars()
                    .map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '-' } else { c })
                    .collect();
                let clean = clean.trim().trim_start_matches('.').trim();
                if clean.is_empty() { "Untitled".to_string() } else { clean.to_string() }
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// A note's tags, with `suspended` added for a suspended card.
fn note_tags(tags: &str, suspended: bool) -> String {
    let has = tags.split_whitespace().any(|t| t.eq_ignore_ascii_case(parser::SUSPENDED_TAG));
    if suspended && !has { format!("{tags} {}", parser::SUSPENDED_TAG) } else { tags.to_string() }
}

/// Add Anki's space-separated note tags as a `#tag` line at the end of the card,
/// before any note that follows it.
fn with_tags(block: &str, tags: &str) -> String {
    // A tag holding `#` would not read back as a tag.
    let line: Vec<String> = tags.split_whitespace().filter(|t| !t.contains('#')).map(|t| format!("#{t}")).collect();
    if line.is_empty() {
        return block.to_string();
    }
    let line = line.join(" ");
    match block.split_once("\n\n") {
        Some((card, note)) => format!("{card}\n{line}\n\n{note}"),
        None => format!("{block}\n{line}"),
    }
}

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn push_block(decks: &mut HashMap<i64, String>, deck: i64, block: &str) {
    let md = decks.entry(deck).or_default();
    if !md.is_empty() {
        md.push('\n');
    }
    md.push_str(block);
    md.push('\n');
}

/// A `Q:`/`A:` card from a normal note type's template, and its amgi id.
fn basic_block(qfmt: &str, afmt: &str, fields: &HashMap<&str, &str>, media: &mut Media) -> Option<(String, String)> {
    let front = text::html_to_markdown(&text::render_template(qfmt, fields), media);
    // The answer side usually repeats the question, through `{{FrontSide}}` or the
    // same fields above `<hr id=answer>`; amgi already shows the question above it.
    let mut back_fields = fields.clone();
    back_fields.insert("FrontSide", "");
    let back_html = text::render_template(afmt, &back_fields);
    let back = text::html_to_markdown(answer_part(&back_html), media);
    let back = match back.strip_prefix(&front) {
        Some(rest) if !rest.trim().is_empty() => rest.trim().to_string(),
        _ => back,
    };
    if front.is_empty() || back.is_empty() {
        return None;
    }
    let block = format!("Q: {front}\nA: {back}");
    let card = single(parser::parse(&block).cards)?;
    Some((block, card.id))
}

/// The part of a rendered answer after Anki's `<hr id=answer>` marker, or all of it.
fn answer_part(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(start) = lower[from..].find("<hr").map(|i| i + from) {
        let Some(end) = lower[start..].find('>').map(|i| i + start) else { break };
        let tag: String = lower[start..end].chars().filter(|c| !matches!(c, '"' | '\'' | ' ')).collect();
        if tag.contains("id=answer") {
            return &html[end + 1..];
        }
        from = end;
    }
    html
}

/// A `C:` card from a cloze note, with the amgi id and Anki cloze number of each
/// deletion. The rest of the answer side, such as Back Extra, follows as a note.
fn cloze_block(
    notetype: &Notetype,
    fields: &HashMap<&str, &str>,
    active: &BTreeSet<u32>,
    media: &mut Media,
) -> Option<(String, Vec<(String, u32)>)> {
    let (_, qfmt, afmt) = notetype.templates.first()?;
    // Image occlusion masks are clozes over an image; there is nothing to show without it.
    if qfmt.contains("image-occlusion") {
        return None;
    }
    let cloze_fields = cloze_fields(qfmt);
    // Render only the cloze fields, the rest would show on the question side.
    let source = cloze_fields
        .iter()
        .filter_map(|name| fields.get(name.as_str()))
        .map(|html| text::html_to_markdown(html, media))
        .collect::<Vec<_>>()
        .join("\n");
    let (cloze, numbers) = text::convert_clozes(&source, |n| active.contains(&n));
    if numbers.is_empty() {
        return None;
    }
    let mut block = format!("C: {cloze}");
    let cards = parser::parse(&block).cards;
    if cards.len() != numbers.len() {
        return None;
    }
    let mut extra_fields = fields.clone();
    for name in cloze_fields.iter().map(String::as_str).chain(["FrontSide"]) {
        extra_fields.insert(name, "");
    }
    let extra = text::html_to_markdown(&text::render_template(afmt, &extra_fields), media);
    if !extra.is_empty() {
        block = format!("{block}\n\n{extra}");
    }
    Some((block, cards.into_iter().map(|c| c.id).zip(numbers).collect()))
}

/// The amgi cards for Anki cloze card `ord`, which covers every `c{ord+1}` deletion.
fn ids_for_cloze(ids: &[(String, u32)], ord: u32) -> Vec<String> {
    ids.iter().filter(|(_, n)| *n == ord + 1).map(|(id, _)| id.clone()).collect()
}

/// Field names used with the `cloze:` filter in a template.
fn cloze_fields(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else { break };
        let tag = &rest[start + 2..start + 2 + len];
        if tag.split(':').rev().skip(1).any(|f| f.trim() == "cloze") {
            names.push(tag.rsplit(':').next().unwrap_or_default().trim().to_string());
        }
        rest = &rest[start + 2 + len + 2..];
    }
    names
}

fn single<T>(items: Vec<T>) -> Option<T> {
    let mut items = items.into_iter();
    let first = items.next()?;
    items.next().is_none().then_some(first)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// Write a legacy `.apkg` holding [`legacy_collection`] to `path`.
    pub(crate) fn write_package(path: &Path) {
        let db = path.with_extension("anki2");
        let _ = fs::remove_file(&db);
        legacy_collection().execute("VACUUM INTO ?1", [db.to_str().unwrap()]).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("collection.anki21", options).unwrap();
        zip.write_all(&fs::read(&db).unwrap()).unwrap();
        zip.start_file("media", options).unwrap();
        zip.write_all(br#"{"0": "hi.png", "1": "paris.mp3"}"#).unwrap();
        zip.start_file("0", options).unwrap();
        zip.write_all(b"png bytes").unwrap();
        zip.start_file("1", options).unwrap();
        zip.write_all(b"mp3 bytes").unwrap();
        zip.finish().unwrap();
        fs::remove_file(db).unwrap();
    }

    fn legacy_collection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        let models = r#"{
            "1": {"name": "Basic (and reversed card)", "type": 0,
                  "flds": [{"name": "Front", "ord": 0}, {"name": "Back", "ord": 1}],
                  "tmpls": [{"name": "Card 1", "ord": 0, "qfmt": "{{Front}}", "afmt": "{{FrontSide}}<hr id=answer>{{Back}}"},
                            {"name": "Card 2", "ord": 1, "qfmt": "{{Back}}", "afmt": "{{FrontSide}}<hr id=answer>{{Front}}"}]},
            "2": {"name": "Cloze", "type": 1,
                  "flds": [{"name": "Text", "ord": 0}, {"name": "Back Extra", "ord": 1}],
                  "tmpls": [{"name": "Cloze", "ord": 0, "qfmt": "{{cloze:Text}}", "afmt": "{{cloze:Text}}<br>{{Back Extra}}"}]}
        }"#;
        let decks = r#"{"1": {"name": "Default"}, "10": {"name": "Lang::Kazakh/Verbs"}}"#;
        conn.execute_batch(
            "CREATE TABLE col (decks TEXT, models TEXT);
             CREATE TABLE notes (id INTEGER, mid INTEGER, flds TEXT, tags TEXT);
             CREATE TABLE cards (id INTEGER, nid INTEGER, did INTEGER, odid INTEGER, ord INTEGER, type INTEGER, queue INTEGER);
             CREATE TABLE revlog (id INTEGER, cid INTEGER, ease INTEGER, type INTEGER, ivl INTEGER, factor INTEGER);",
        )
        .unwrap();
        conn.execute("INSERT INTO col VALUES (?1, ?2)", [decks, models]).unwrap();
        conn.execute_batch(
            "INSERT INTO notes VALUES
               (100, 1, 'сәлем' || char(31) || 'hello<img src=hi.png>', ''),
               (101, 2, 'The {{c1::capital}} of [x] is {{c2::Paris}}[sound:paris.mp3]' || char(31) || 'extra[sound:gone.mp3]', 'geo'),
               (102, 2, 'no deletions' || char(31) || '', ''),
               (103, 1, 'Capital of Spain?' || char(31) || 'Madrid', 'geo');
             INSERT INTO cards VALUES
               (1000, 100, 10, 0, 0, 2, 2), (1001, 100, 99, 10, 1, 0, 0),
               (1010, 101, 1, 0, 0, 2, 2), (1011, 101, 1, 0, 1, 0, -1),
               (1020, 102, 1, 0, 0, 0, 0),
               (1030, 103, 1, 0, 0, 2, -1);
             INSERT INTO revlog VALUES
               (1700000000000, 1000, 3, 0, 1, 2500), (1700086400000, 1000, 3, 1, 3, 2500),
               (1700000000000, 1010, 1, 0, -600, 2500),
               (1700000000000, 1030, 3, 0, 1, 2500);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn converts_legacy_collection() {
        let pkg = Package::from_db(&legacy_collection()).unwrap();
        let out = pkg.convert();
        let paths: Vec<_> = out.decks.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["Default", "Lang/Kazakh-Verbs"]);
        // `c2` is suspended, so Paris is no longer a gap.
        // `c2` is suspended, so it gets a tagged copy of the cloze of its own. A suspended
        // card keeps its history and is tagged.
        assert_eq!(
            out.decks[0].1,
            "C: The [capital] of \\[x\\] is Paris![](media/paris.mp3)\n#geo\n\nextra![](media/gone.mp3)\n\n\
             C: The capital of \\[x\\] is [Paris]![](media/paris.mp3)\n#geo #suspended\n\nextra![](media/gone.mp3)\n\n\
             Q: Capital of Spain?\nA: Madrid\n#geo #suspended\n"
        );
        assert_eq!(
            out.decks[1].1,
            "+++\nname = \"Kazakh/Verbs\"\n+++\n\nQ: сәлем\nA: hello![](../media/hi.png)\n\nQ: hello![](../media/hi.png)\nA: сәлем\n"
        );
        assert_eq!(out.media.images.iter().collect::<Vec<_>>(), ["hi.png"]);
        assert_eq!(out.media.sounds.iter().collect::<Vec<_>>(), ["gone.mp3", "paris.mp3"]);
        assert_eq!((out.notes, out.cards, out.skipped_notes, out.suspended_cards), (4, 5, 1, 2));

        let all = parser::parse(&out.decks[1].1).cards;
        assert_eq!(out.histories.len(), 3);
        assert_eq!(out.histories[0].0, all[0].id);
        assert_eq!(out.histories[0].1.len(), 2);
        assert_eq!(out.histories[0].1[1], Revlog { time: 1_700_086_400, ease: 3, kind: 1, interval: 3, factor: 2500 });
        // The cloze history belongs to the `c1` deletion only.
        let cloze = parser::parse(&out.decks[0].1).cards;
        assert_eq!(out.histories[1].0, cloze[0].id);
    }

    #[test]
    fn protobuf() {
        // kind = 1 (varint), css = "ab" (bytes), field 3 fixed32.
        let buf = [0x08, 0x01, 0x1a, 0x02, b'a', b'b', 0x1d, 0, 0, 0, 0];
        let fields: Vec<_> = proto_fields(&buf).collect();
        assert_eq!(fields.len(), 3);
        assert!(matches!(fields[0], (1, Proto::Varint(1))));
        assert!(matches!(fields[1], (3, Proto::Bytes(b"ab"))));
        assert_eq!(proto_fields(&[0x0a, 0x05, b'x']).count(), 0);
    }

    #[test]
    fn answer_side_drops_repeated_question() {
        let fields = HashMap::from([("Front", "q"), ("Back", "a")]);
        let block = |afmt| basic_block("{{Front}}", afmt, &fields, &mut Media::default()).unwrap().0;
        assert_eq!(block("{{Front}}<HR id=\"answer\">{{Back}}"), "Q: q\nA: a");
        assert_eq!(block("{{Front}}<br>{{Back}}"), "Q: q\nA: a");
        assert_eq!(block("{{Front}}"), "Q: q\nA: q");
    }

    #[test]
    fn media_indexes() {
        let legacy = media_index(br#"{"0": "a.png", "1": "b c.jpg"}"#).unwrap();
        assert_eq!(legacy["b c.jpg"], "1");
        // Two entries: "a.png" stored as "0", "b.png" with legacy_zip_filename 7.
        let modern = [0x0a, 0x07, 0x0a, 0x05, b'a', b'.', b'p', b'n', b'g', 0x0a, 0x0a, 0x0a, 0x05, b'b', b'.', b'p', b'n', b'g', 0xf8, 0x0f, 0x07];
        let modern = media_index(&modern).unwrap();
        assert_eq!((modern["a.png"].as_str(), modern["b.png"].as_str()), ("0", "7"));
    }

    #[test]
    fn cloze_field_names() {
        assert_eq!(cloze_fields("{{cloze:Text}}<br>{{Extra}}{{cloze:Other}}"), ["Text", "Other"]);
    }
}
