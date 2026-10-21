//! Reading Anki packages (`.apkg`, `.colpkg`) and converting them to Markdown decks.
//!
//! A package is a zip archive holding the collection as SQLite: `collection.anki21b`
//! (zstd-compressed, schema 18, current Anki) or `collection.anki21` / `collection.anki2`
//! (legacy schema 11, where note types and decks are JSON in the `col` table).
//! Images and sounds used by cards are copied to a `media` folder.
//!
//! This module only uses the collection's public API: it converts a package to
//! Markdown and plain review history, and hands both to [`CardCollection::import`].

mod text;

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::{parser, AmgiError, CardCollection, HistoryEntry, Import, Rating, ReviewState, MEDIA_DIR};
pub use text::Media;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ImportSummary {
    /// Deck files written, relative to the collection root.
    pub files: Vec<String>,
    pub notes: u32,
    pub cards: u32,
    /// Cards that kept their Anki review history.
    pub reviewed_cards: u32,
    /// Cards that already had review history in this collection, which was kept.
    pub existing_history: u32,
    /// Cards never reviewed, in Anki or here.
    pub new_cards: u32,
    /// Notes that produced no card, such as image occlusion notes.
    pub skipped_notes: u32,
    /// Suspended Anki cards, imported with the `#suspended` tag.
    pub suspended_cards: u32,
    /// Images now in the collection's `media` folder.
    pub images: u32,
    /// Images cards refer to that the package does not contain.
    pub missing_images: u32,
    /// Sounds now in the collection's `media` folder.
    pub sounds: u32,
    /// Sounds cards refer to that the package does not contain.
    pub missing_sounds: u32,
}

#[uniffi::export]
impl CardCollection {
    /// Import an Anki package (`.apkg` or `.colpkg`) as Markdown decks in this
    /// collection, keeping review history. Images and sounds go to a `media` folder
    /// at the root. Existing files are never overwritten.
    pub fn import_anki(&self, path: String) -> Result<ImportSummary, AmgiError> {
        let package = Package::read(Path::new(&path))?;
        let conversion = package.convert();
        let media_dir = self.root().join(MEDIA_DIR);
        let images = package.write_media(&conversion.media.images, &media_dir)?;
        let sounds = package.write_media(&conversion.media.sounds, &media_dir)?;
        let histories = conversion.histories.iter().map(History::entries).collect();
        let imported = self.import(Import { decks: conversion.decks, note_types: conversion.note_types, histories })?;
        Ok(ImportSummary {
            files: imported.files,
            notes: conversion.notes,
            cards: conversion.cards,
            reviewed_cards: imported.reviewed_cards,
            existing_history: imported.existing_history,
            new_cards: imported.new_cards,
            skipped_notes: conversion.skipped_notes,
            suspended_cards: conversion.suspended_cards,
            images,
            missing_images: conversion.media.images.len() as u32 - images,
            sounds,
            missing_sounds: conversion.media.sounds.len() as u32 - sounds,
        })
    }
}

impl From<Error> for AmgiError {
    fn from(e: Error) -> Self {
        AmgiError::Import { message: e.to_string() }
    }
}

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
    /// When Anki has the card due next (Unix seconds); `None` for new cards.
    due: Option<i64>,
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

    /// The entry as amgi history. `None` for entries that left the schedule alone,
    /// such as filtered-deck reviews with rescheduling off.
    pub fn history(&self) -> Option<HistoryEntry> {
        let rating = match self.ease {
            1 => Some(Rating::Again),
            2 => Some(Rating::Hard),
            3 => Some(Rating::Good),
            4 => Some(Rating::Easy),
            _ => None,
        };
        let time = self.time;
        match (rating, self.kind) {
            (Some(_), 3) if self.factor == 0 => None,
            (Some(rating), 0..=3) => Some(HistoryEntry::Review {
                time,
                rating,
                // A filtered-deck review doesn't say which stage the card was in.
                state: match self.kind {
                    0 => Some(ReviewState::Learning),
                    1 => Some(ReviewState::Review),
                    2 => Some(ReviewState::Relearning),
                    _ => None,
                },
                interval_secs: (self.interval != 0).then(|| self.interval_secs()),
            }),
            // Manual entries: "Forget" resets the card, "Set Due Date" and FSRS
            // rescheduling move the due date.
            (None, 4) if self.interval == 0 => Some(HistoryEntry::Reset { time }),
            (None, 4 | 5) => Some(HistoryEntry::Reschedule { time, interval_secs: self.interval_secs() }),
            _ => None,
        }
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
        let timing = Timing::read(conn, modern)?;
        // Cards in a filtered deck remember their home deck in `odid` and their own
        // due date in `odue`.
        let cards = conn
            .prepare(
                "SELECT id, nid, CASE WHEN odid != 0 THEN odid ELSE did END, ord, type, queue,
                        CASE WHEN odid != 0 AND odue != 0 THEN odue ELSE due END
                 FROM cards ORDER BY nid, ord",
            )?
            .query_map([], |r| {
                let kind: i64 = r.get(4)?;
                Ok(AnkiCard {
                    id: r.get(0)?,
                    note: r.get(1)?,
                    deck: r.get(2)?,
                    ord: r.get(3)?,
                    is_new: kind == 0,
                    suspended: r.get::<_, i64>(5)? == -1,
                    due: timing.due(kind, r.get(6)?),
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

/// An amgi card's history, from the Anki card it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    /// amgi card id.
    pub id: String,
    pub log: Vec<Revlog>,
    /// When Anki has the card due (Unix seconds).
    pub due: Option<i64>,
}

impl History {
    /// The log as amgi history, ending at Anki's due date. Anki can move a due date
    /// without logging it, as rescheduling after optimizing FSRS does, so the card's
    /// own date is the one to keep.
    fn entries(&self) -> (String, Vec<HistoryEntry>) {
        let mut entries: Vec<HistoryEntry> = self.log.iter().filter_map(Revlog::history).collect();
        if let (Some(due), Some(last)) = (self.due, self.log.last()) {
            entries.push(HistoryEntry::Reschedule { time: last.time, interval_secs: due - last.time });
        }
        (self.id.clone(), entries)
    }
}

/// What Anki's day numbers mean: review cards are due on a day counted from the
/// collection's creation, and a day starts at the rollover hour.
struct Timing {
    /// Collection creation, Unix seconds.
    created: i64,
    /// Minutes west of UTC when the collection was created and now; absent in
    /// collections from before Anki 2.1.
    offsets: Option<(i64, i64)>,
    rollover_hour: i64,
}

impl Timing {
    fn read(conn: &Connection, modern: bool) -> Result<Self> {
        let created: i64 = conn.query_row("SELECT crt FROM col", [], |r| r.get(0))?;
        let config: HashMap<String, serde_json::Value> = if modern {
            let mut stmt = conn.prepare(
                "SELECT key, val FROM config WHERE key IN ('creationOffset', 'localOffset', 'rollover')",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
            let mut config = HashMap::new();
            for row in rows {
                let (key, val) = row?;
                config.insert(key, serde_json::from_slice(&val)?);
            }
            config
        } else {
            let conf: String = conn.query_row("SELECT conf FROM col", [], |r| r.get(0))?;
            serde_json::from_str(&conf)?
        };
        let int = |key: &str| config.get(key).and_then(serde_json::Value::as_i64);
        Ok(Timing {
            created,
            offsets: int("creationOffset").zip(int("localOffset")),
            rollover_hour: int("rollover").unwrap_or(4),
        })
    }

    /// A card's `due` as Unix seconds: a time for cards in learning, a day number
    /// otherwise. `None` for new cards.
    fn due(&self, kind: i64, due: i64) -> Option<i64> {
        match kind {
            0 => None,
            // Learning cards due later than today hold a day number too.
            1 | 3 if due > 1_000_000_000 => Some(due),
            _ => Some(self.day_start(due)),
        }
    }

    /// When Anki's day `day` starts.
    fn day_start(&self, day: i64) -> i64 {
        let rollover = self.rollover_hour * 3600;
        match self.offsets {
            // The day count runs from the creation date in the time zone of the time.
            Some((created_west, local_west)) => {
                let first = (self.created - created_west * 60 - rollover).div_euclid(86_400);
                (first + day) * 86_400 + rollover + local_west * 60
            }
            // Older collections start their days at creation time.
            None => self.created + day * 86_400,
        }
    }
}

/// A converted package: Markdown per deck, plus the review history of each card.
#[derive(Debug, Default)]
pub struct Conversion {
    /// Deck path (`Languages/Kazakh`) and Markdown, in Anki deck order.
    pub decks: Vec<(String, String)>,
    /// The review history of each card, from the Anki card it came from. Cards that
    /// were new in Anki are absent.
    pub histories: Vec<History>,
    pub notes: u32,
    pub cards: u32,
    /// Notes that produced no card, such as image occlusion or empty templates.
    pub skipped_notes: u32,
    /// Suspended Anki cards, written with the `#suspended` tag.
    pub suspended_cards: u32,
    pub media: Media,
    /// Note type name and its file's TOML, for the note types decks use.
    pub note_types: Vec<(String, String)>,
}

impl Package {
    pub fn convert(&self) -> Conversion {
        let mut out = Conversion::default();
        let mut decks: HashMap<i64, String> = HashMap::new();
        let mut cards_by_note: HashMap<i64, Vec<&AnkiCard>> = HashMap::new();
        for card in &self.cards {
            cards_by_note.entry(card.note).or_default().push(card);
        }
        let mut layouts: HashMap<i64, NoteLayout> = HashMap::new();
        let mut ids: Vec<i64> = self.notetypes.keys().copied().collect();
        ids.sort();
        for id in ids {
            let Some(mut layout) = note_layout(&self.notetypes[&id]) else { continue };
            // Names are files, so two note types can't share one, whatever its case.
            let base = clean_part(&self.notetypes[&id].name);
            layout.name = (1..)
                .map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") })
                .find(|name| !layouts.values().any(|l| l.name.eq_ignore_ascii_case(name)))
                .expect("unbounded range");
            layouts.insert(id, layout);
        }
        let mut types = parser::NoteTypes::default();
        for layout in layouts.values() {
            types.insert(&layout.name, &layout.toml());
        }
        // The note types notes were written with.
        let mut used: BTreeSet<&str> = BTreeSet::new();

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
                    push_block(decks.entry(first.deck).or_default(), &block);
                }
            } else if let Some(layout) = layouts.get(&note.notetype)
                && let Some(block) = self.note_block(layout, &types, note, cards, &mut out)
            {
                push_block(decks.entry(cards[0].deck).or_default(), &block);
                used.insert(&layout.name);
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
                    push_block(decks.entry(card.deck).or_default(), &block);
                }
            }
            if out.cards == before {
                out.skipped_notes += 1;
            }
        }

        let mut order: Vec<i64> = decks.keys().copied().collect();
        order.sort_by_key(|id| self.decks.get(id));
        for id in order {
            let path = self.deck_path(id);
            let name = self.decks.get(&id).and_then(|parts| parts.last()).cloned().unwrap_or_default();
            let markdown = decks.remove(&id).unwrap_or_default();
            out.decks.push((path.clone(), deck_file(&path, &name, &markdown)));
        }
        out.note_types = layouts.values().filter(|l| used.contains(l.name.as_str())).map(|l| (l.name.clone(), l.toml())).collect();
        out.note_types.sort();
        out
    }

    /// A note as its `@` line and `Field: value` lines, when it makes the same cards as
    /// in Anki: all in one deck, suspended alike, and one per Anki card. Adds its cards' histories and
    /// media to `out`; `None` leaves `out` as it was.
    fn note_block(
        &self,
        layout: &NoteLayout,
        types: &parser::NoteTypes,
        note: &Note,
        cards: &[&AnkiCard],
        out: &mut Conversion,
    ) -> Option<String> {
        let (deck, suspended) = (cards.first()?.deck, cards[0].suspended);
        if cards.iter().any(|c| c.deck != deck || c.suspended != suspended) {
            return None;
        }
        let mut media = Media { prefix: self.media_prefix(deck), ..Media::default() };
        let values: Vec<String> = note.fields.iter().map(|html| text::html_to_markdown(html, &mut media)).collect();
        let note_type = types.get(&layout.name)?;
        let mut block = format!("@{}\n", layout.name);
        for (name, value) in layout.fields.iter().zip(&values).filter(|(_, v)| !v.is_empty()) {
            let mut lines = value.lines();
            writeln!(block, "{name}: {}", lines.next().unwrap_or_default()).unwrap();
            for line in lines {
                let mut line = line.to_string();
                parser::escape_line(&mut line, note_type);
                writeln!(block, "{line}").unwrap();
            }
        }
        let block = with_tags(block.trim_end(), &note_tags(&note.tags, suspended));

        let filled = |f: &usize| values.get(*f).is_some_and(|v| !v.is_empty());
        let made: Vec<u32> = (0..layout.rules.len() as u32)
            .filter(|&ord| {
                let rule = &layout.rules[ord as usize];
                rule.front.iter().any(filled) && rule.back.iter().any(filled)
            })
            .collect();
        let anki: BTreeSet<u32> = cards.iter().map(|c| c.ord).collect();
        let parsed = parser::parse_deck(&block, types);
        if !parsed.warnings.is_empty() || parsed.cards.len() != made.len() || made.iter().copied().collect::<BTreeSet<_>>() != anki {
            return None;
        }
        let ids: HashMap<u32, String> = made.into_iter().zip(parsed.cards.into_iter().map(|c| c.id)).collect();
        self.add_histories(cards, |ord| ids.get(&ord).cloned().into_iter().collect(), out);
        out.cards += cards.len() as u32;
        if suspended {
            out.suspended_cards += cards.len() as u32;
        }
        out.media.images.extend(media.images);
        out.media.sounds.extend(media.sounds);
        Some(block)
    }

    fn add_histories(&self, cards: &[&AnkiCard], ids: impl Fn(u32) -> Vec<String>, out: &mut Conversion) {
        for card in cards.iter().filter(|c| !c.is_new) {
            if let Some(log) = self.revlog.get(&card.id) {
                for id in ids(card.ord) {
                    out.histories.push(History { id, log: log.clone(), due: card.due });
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
        parts.iter().map(|part| clean_part(part)).collect::<Vec<_>>().join("/")
    }
}

/// A deck or note type name as part of a file path, safe for any file system and
/// never hidden.
fn clean_part(part: &str) -> String {
    let clean: String = part
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '-' } else { c })
        .collect();
    let clean = clean.trim().trim_start_matches('.').trim();
    if clean.is_empty() { "Untitled".to_string() } else { clean.to_string() }
}

/// A deck file's Markdown, with front matter for its Anki name when the file name had
/// to change.
fn deck_file(path: &str, name: &str, markdown: &str) -> String {
    if Some(name) == path.rsplit('/').next() {
        return markdown.to_string();
    }
    format!("+++\nname = {}\n+++\n\n{markdown}", parser::toml_string(name))
}

/// The fields and card rules an Anki note type becomes: one rule per template, with
/// the fields its question shows on the front and those its answer adds on the back.
struct NoteLayout {
    /// Its file name in `note-types`, unique among the package's note types.
    name: String,
    fields: Vec<String>,
    /// Indexed by template ordinal.
    rules: Vec<LayoutRule>,
}

struct LayoutRule {
    front: Vec<usize>,
    back: Vec<usize>,
    /// The question has a typing box: the first back field is typed in.
    typed: bool,
}

impl NoteLayout {
    /// Its note type file.
    fn toml(&self) -> String {
        let list = |fields: &[usize]| fields.iter().map(|&f| self.fields[f].as_str()).collect::<Vec<_>>().join(", ");
        let names: Vec<String> = self.fields.iter().map(|f| parser::toml_string(f)).collect();
        let mut out = format!("fields = [{}]\ncards = [\n", names.join(", "));
        for rule in &self.rules {
            let arrow = if rule.typed { "=>" } else { "->" };
            let rule = format!("{} {arrow} {}", list(&rule.front), list(&rule.back));
            writeln!(out, "  {},", parser::toml_string(&rule)).unwrap();
        }
        out.push_str("]\n");
        out
    }
}

/// How a normal note type's notes can be written with fields, or `None` to render
/// its cards as `Q:`/`A:` text as before. That is kept for note types of two fields
/// or fewer, which read better that way, and for those whose questions hold text of
/// their own, such as `Plural of {{Word}}?`, which fields would lose.
fn note_layout(notetype: &Notetype) -> Option<NoteLayout> {
    if notetype.cloze || notetype.fields.len() <= 2 || notetype.templates.is_empty() {
        return None;
    }
    let fields: Vec<String> = notetype.fields.iter().map(|f| f.trim().to_string()).collect();
    let usable = |(i, f): (usize, &String)| {
        !f.is_empty() && !f.contains([':', ',']) && !f.contains("->") && !["Q", "A", "C"].contains(&f.as_str()) && !fields[..i].contains(f)
    };
    if !fields.iter().enumerate().all(usable) {
        return None;
    }
    let indexes = |names: Vec<String>, skip: &[usize]| {
        let mut out: Vec<usize> = Vec::new();
        for i in names.iter().filter_map(|n| fields.iter().position(|f| f == n)) {
            if !out.contains(&i) && !skip.contains(&i) {
                out.push(i);
            }
        }
        out
    };
    let mut rules: Vec<LayoutRule> = Vec::new();
    for (_, qfmt, afmt) in &notetype.templates {
        if has_own_text(qfmt) {
            return None;
        }
        let front = indexes(shown_fields(qfmt, true)?, &[]);
        let answer = shown_fields(&text::answer_template(afmt), false)?;
        let mut back = indexes(answer.clone(), &front);
        // Fields the answer shows right after a question's field, such as a word's
        // audio after the word, come first, rather than beside some other field.
        let follows_front = |f: &usize| {
            let at = answer.iter().position(|n| *n == fields[*f]);
            at.is_some_and(|at| at > 0 && fields.iter().position(|n| *n == answer[at - 1]).is_some_and(|p| front.contains(&p)))
        };
        let (mut first, rest): (Vec<usize>, Vec<usize>) = back.iter().partition(|f| follows_front(f));
        first.extend(rest);
        back = first;
        // The field typed in leads the answer.
        let typed = typed_field(qfmt).and_then(|name| fields.iter().position(|f| *f == name));
        if let Some(t) = typed {
            back.retain(|&f| f != t);
            back.insert(0, t);
        }
        if front.is_empty() || back.is_empty() || rules.iter().any(|r| r.front == front) {
            return None;
        }
        rules.push(LayoutRule { front, back, typed: typed.is_some() });
    }
    Some(NoteLayout { name: String::new(), fields, rules })
}

/// The fields a template shows, in order, or `None` when a fixed list can't say: a
/// field in an HTML attribute, and on the question side any script, or a
/// field shown only when another one is filled, as in `{{#IsSentenceCard}}{{Sentence}}`.
/// Conditions, typing boxes, text-to-speech and Anki's own values such as
/// `{{FrontSide}}` are left out, as are hints on the question side: Anki hides them
/// behind a link, and makes the card without them.
fn shown_fields(template: &str, front: bool) -> Option<Vec<String>> {
    let lower = template.to_ascii_lowercase();
    if front && lower.contains("<script") {
        return None;
    }
    let mut names = Vec::new();
    let mut sections: Vec<&str> = Vec::new();
    let (mut in_tag, mut in_script) = (false, false);
    let mut i = 0;
    while i < template.len() {
        let rest = &template[i..];
        if let Some(inner) = rest.strip_prefix("{{") {
            let len = inner.find("}}")?;
            let tag = inner[..len].trim();
            i += len + 4;
            // A script on the answer side only does something with the field.
            if in_script {
                continue;
            }
            if in_tag {
                return None;
            }
            if let Some(name) = tag.strip_prefix(['#', '^']) {
                sections.push(name.trim());
                continue;
            }
            if tag.starts_with('/') {
                sections.pop();
                continue;
            }
            let mut parts = tag.rsplit(':');
            let name = parts.next().unwrap_or_default().trim();
            let hidden = parts.map(str::trim).any(|f| {
                f.starts_with("type") || f.starts_with("tts") || f.starts_with("cloze") || (f == "hint" && front)
            });
            let special = ["FrontSide", "Tags", "Type", "Deck", "Subdeck", "Card", "CardFlag"].contains(&name);
            if hidden || special {
                continue;
            }
            if front && sections.iter().any(|s| *s != name) {
                return None;
            }
            names.push(name.to_string());
            continue;
        }
        let lower_rest = &lower[i..];
        if in_script {
            in_script = !lower_rest.starts_with("</script");
        } else if lower_rest.starts_with("<script") {
            in_script = true;
        }
        match rest.as_bytes()[0] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            _ => {}
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    Some(names)
}

/// The field a question's typing box, `{{type:Word}}` or `{{type:nc:Word}}`, asks for.
fn typed_field(template: &str) -> Option<String> {
    let start = template.find("{{type:")? + "{{type:".len();
    let tag = &template[start..start + template[start..].find("}}")?];
    Some(tag.strip_prefix("nc:").unwrap_or(tag).trim().to_string())
}

/// Whether a template shows words or numbers besides its fields.
fn has_own_text(template: &str) -> bool {
    let mut bare = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        bare.push_str(&rest[..start]);
        let Some(len) = rest[start + 2..].find("}}") else { break };
        rest = &rest[start + 2 + len + 2..];
    }
    bare.push_str(rest);
    let shown = text::html_to_markdown(&bare, &mut Media::default());
    // Entities such as `&mdash;` are punctuation, not words.
    let mut words = shown.split('&').enumerate().map(|(i, part)| match part.split_once(';') {
        Some((name, after)) if i > 0 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '#') => after,
        _ => part,
    });
    words.any(|part| part.chars().any(char::is_alphanumeric))
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

fn push_block(md: &mut String, block: &str) {
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
    let back_html = text::render_template(&text::answer_template(afmt), &back_fields);
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
            "CREATE TABLE col (crt INTEGER, conf TEXT, decks TEXT, models TEXT);
             CREATE TABLE notes (id INTEGER, mid INTEGER, flds TEXT, tags TEXT);
             CREATE TABLE cards (id INTEGER, nid INTEGER, did INTEGER, odid INTEGER, ord INTEGER, type INTEGER, queue INTEGER, due INTEGER, odue INTEGER);
             CREATE TABLE revlog (id INTEGER, cid INTEGER, ease INTEGER, type INTEGER, ivl INTEGER, factor INTEGER);",
        )
        .unwrap();
        // No time zones in `conf`: days count from `crt`, as in collections from before Anki 2.1.
        conn.execute("INSERT INTO col VALUES (1699920000, '{}', ?1, ?2)", [decks, models]).unwrap();
        conn.execute_batch(
            "INSERT INTO notes VALUES
               (100, 1, 'сәлем' || char(31) || 'hello<img src=hi.png>', ''),
               (101, 2, 'The {{c1::capital}} of [x] is {{c2::Paris}}[sound:paris.mp3]' || char(31) || 'extra[sound:gone.mp3]', 'geo'),
               (102, 2, 'no deletions' || char(31) || '', ''),
               (103, 1, 'Capital of Spain?' || char(31) || 'Madrid', 'geo');
             INSERT INTO cards VALUES
               (1000, 100, 99, 10, 0, 2, 2, -100, 5), (1001, 100, 99, 10, 1, 0, 0, 7, 0),
               (1010, 101, 1, 0, 0, 2, 2, 2, 0), (1011, 101, 1, 0, 1, 0, -1, 8, 0),
               (1020, 102, 1, 0, 0, 0, 0, 9, 0),
               (1030, 103, 1, 0, 0, 2, -1, 4, 0);
             INSERT INTO revlog VALUES
               (1700000000000, 1000, 3, 0, 1, 2500), (1700086400000, 1000, 3, 1, 3, 2500),
               (1700000000000, 1010, 1, 0, -600, 2500),
               (1700000000000, 1030, 3, 0, 1, 2500);",
        )
        .unwrap();
        conn
    }

    /// A legacy collection of `notes` and `cards` rows in deck 20, `Spanish`, with
    /// note types of more than two fields.
    fn vocab_collection(notes: &str, cards: &str, revlog: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        let models = r#"{
            "3": {"name": "Vocab", "type": 0,
                  "flds": [{"name": "Word", "ord": 0}, {"name": "Meaning", "ord": 1}, {"name": "Audio", "ord": 2}, {"name": "Sentence", "ord": 3}],
                  "tmpls": [{"name": "Recognition", "ord": 0, "qfmt": "<div class=big>{{Word}}</div>{{Audio}}{{#Hint}}{{hint:Hint}}{{/Hint}}", "afmt": "{{FrontSide}}<hr id=answer>{{Meaning}}<br>{{Sentence}}"},
                            {"name": "Recall", "ord": 1, "qfmt": "{{Meaning}} {{type:Word}}", "afmt": "{{FrontSide}}<hr id=answer>{{type:Word}} {{Audio}}"}]},
            "4": {"name": "Phrase", "type": 0,
                  "flds": [{"name": "Text", "ord": 0}, {"name": "Translation", "ord": 1}, {"name": "Notes", "ord": 2}],
                  "tmpls": [{"name": "Card 1", "ord": 0, "qfmt": "{{Text}}", "afmt": "{{FrontSide}}<hr id=answer>{{Translation}}"}]},
            "5": {"name": "Plural", "type": 0,
                  "flds": [{"name": "Word", "ord": 0}, {"name": "Plural", "ord": 1}, {"name": "Gender", "ord": 2}],
                  "tmpls": [{"name": "Card 1", "ord": 0, "qfmt": "Plural of {{Word}}?", "afmt": "{{FrontSide}}<hr id=answer>{{Plural}}"}]}
        }"#;
        conn.execute_batch(
            "CREATE TABLE col (crt INTEGER, conf TEXT, decks TEXT, models TEXT);
             CREATE TABLE notes (id INTEGER, mid INTEGER, flds TEXT, tags TEXT);
             CREATE TABLE cards (id INTEGER, nid INTEGER, did INTEGER, odid INTEGER, ord INTEGER, type INTEGER, queue INTEGER, due INTEGER, odue INTEGER);
             CREATE TABLE revlog (id INTEGER, cid INTEGER, ease INTEGER, type INTEGER, ivl INTEGER, factor INTEGER);",
        )
        .unwrap();
        conn.execute("INSERT INTO col VALUES (1699920000, '{}', ?1, ?2)", [r#"{"20": {"name": "Spanish"}}"#, models]).unwrap();
        conn.execute_batch(&format!("INSERT INTO notes VALUES {notes}; INSERT INTO cards VALUES {cards}; {revlog}")).unwrap();
        conn
    }

    #[test]
    fn notes_with_many_fields_keep_them() {
        let notes = "(200, 3, 'manzana' || char(31) || 'apple' || char(31) || '[sound:manzana.mp3]' || char(31) || 'Me como una manzana.<br>Meaning: not a field', 'food'),
             (201, 3, 'queso' || char(31) || 'cheese' || char(31) || '' || char(31) || '', ''),
             (202, 4, 'Buenos días' || char(31) || 'Good morning' || char(31) || 'formal', ''),
             (203, 5, 'pan' || char(31) || 'panes' || char(31) || 'm', ''),
             (204, 3, 'uva' || char(31) || 'grape' || char(31) || '' || char(31) || '', '')";
        let cards = "(2000, 200, 20, 0, 0, 2, 2, 5, 0), (2001, 200, 20, 0, 1, 2, 2, 6, 0),
             (2010, 201, 20, 0, 0, 0, 0, 1, 0), (2011, 201, 20, 0, 1, 0, 0, 2, 0),
             (2020, 202, 20, 0, 0, 0, 0, 3, 0),
             (2030, 203, 20, 0, 0, 0, 0, 4, 0),
             (2040, 204, 20, 0, 0, 0, -1, 7, 0), (2041, 204, 20, 0, 1, 0, 0, 8, 0)";
        let revlog = "INSERT INTO revlog VALUES (1700000000000, 2001, 3, 0, 1, 2500);";
        let out = Package::from_db(&vocab_collection(notes, cards, revlog)).unwrap().convert();
        let paths: Vec<_> = out.decks.iter().map(|(p, _)| p.as_str()).collect();
        // One file for the deck, whatever note types its notes have.
        assert_eq!(paths, ["Spanish"]);
        assert_eq!(
            out.decks[0].1,
            "@Vocab\nWord: manzana\nMeaning: apple\nAudio: ![](media/manzana.mp3)\nSentence: Me como una manzana.\nMeaning\\: not a field\n#food\n\n\
             @Vocab\nWord: queso\nMeaning: cheese\n\n\
             @Phrase\nText: Buenos días\nTranslation: Good morning\nNotes: formal\n\n\
             Q: Plural of pan?\nA: panes\n\n\
             Q: uva\nA: grape\n#suspended\n\n\
             Q: grape\nA: uva\n"
        );
        let types: Vec<_> = out.note_types.iter().map(|(name, toml)| format!("{name}.toml\n{toml}")).collect();
        assert_eq!(
            types,
            [
                "Phrase.toml\nfields = [\"Text\", \"Translation\", \"Notes\"]\ncards = [\n  \"Text -> Translation\",\n]\n",
                "Vocab.toml\nfields = [\"Word\", \"Meaning\", \"Audio\", \"Sentence\"]\ncards = [\n  \"Word, Audio -> Meaning, Sentence\",\n  \"Meaning => Word, Audio\",\n]\n",
            ]
        );
        assert_eq!((out.notes, out.cards, out.skipped_notes, out.suspended_cards), (5, 8, 0, 1));
        assert_eq!(out.media.sounds.iter().collect::<Vec<_>>(), ["manzana.mp3"]);

        let mut types = parser::NoteTypes::default();
        for (name, toml) in &out.note_types {
            types.insert(name, toml);
        }
        let parsed = parser::parse_deck(&out.decks[0].1, &types);
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        let fronts: Vec<_> = parsed.cards.iter().map(|c| c.front.as_str()).collect();
        assert_eq!(fronts, ["manzana\n\n![](media/manzana.mp3)", "apple", "queso", "cheese", "Buenos días", "Plural of pan?", "uva", "grape"]);
        // The review of the Recall card went to the `Meaning -> Word, Audio` card.
        assert_eq!(out.histories.len(), 1);
        assert_eq!(out.histories[0].id, parsed.cards[1].id);
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
        assert_eq!(out.histories[0].id, all[0].id);
        assert_eq!(out.histories[0].log.len(), 2);
        assert_eq!(out.histories[0].log[1], Revlog { time: 1_700_086_400, ease: 3, kind: 1, interval: 3, factor: 2500 });
        // Day 5 of the collection: the card was in a filtered deck, which keeps its
        // own due date in `odue`.
        assert_eq!(out.histories[0].due, Some(1_699_920_000 + 5 * 86_400));
        // The cloze history belongs to the `c1` deletion only.
        let cloze = parser::parse(&out.decks[0].1).cards;
        assert_eq!(out.histories[1].id, cloze[0].id);
    }

    #[test]
    fn history_ends_at_anki_due_date() {
        let review = Revlog { time: 1_000, ease: 3, kind: 1, interval: 3, factor: 2500 };
        let history = History { id: "a".into(), log: vec![review], due: Some(1_000 + 10 * 86_400) };
        let (_, entries) = history.entries();
        // Anki moved the card from 3 days out to 10 without logging it.
        assert_eq!(entries[1], HistoryEntry::Reschedule { time: 1_000, interval_secs: 10 * 86_400 });
    }

    #[test]
    fn anki_days() {
        // Created 2025-08-25 22:00 at UTC+4, now at UTC+5, days starting at 2 am.
        let timing = Timing { created: 1_756_144_800, offsets: Some((-240, -300)), rollover_hour: 2 };
        // Day 422 starts on 2026-10-21 at 2 am, UTC+5.
        assert_eq!(timing.day_start(422), 1_792_530_000);
        assert_eq!(timing.due(2, 422), Some(1_792_530_000));
        assert_eq!(timing.due(1, 1_792_000_000), Some(1_792_000_000));
        assert_eq!(timing.due(3, 422), Some(1_792_530_000));
        assert_eq!(timing.due(0, 7), None);
    }

    #[test]
    fn revlog_becomes_history() {
        let entry = |ease, kind, interval, factor| Revlog { time: 100, ease, kind, interval, factor };
        let review = |rating, state, interval_secs| Some(HistoryEntry::Review { time: 100, rating, state, interval_secs });
        assert_eq!(entry(1, 0, -600, 2500).history(), review(Rating::Again, Some(ReviewState::Learning), Some(600)));
        assert_eq!(entry(3, 1, 4, 2500).history(), review(Rating::Good, Some(ReviewState::Review), Some(4 * 86_400)));
        assert_eq!(entry(4, 2, 0, 2500).history(), review(Rating::Easy, Some(ReviewState::Relearning), None));
        // Filtered decks: the stage is unknown, and with rescheduling off nothing changed.
        assert_eq!(entry(2, 3, 5, 2500).history(), review(Rating::Hard, None, Some(5 * 86_400)));
        assert_eq!(entry(2, 3, 5, 0).history(), None);
        // Forget, Set Due Date, FSRS rescheduling.
        assert_eq!(entry(0, 4, 0, 0).history(), Some(HistoryEntry::Reset { time: 100 }));
        assert_eq!(entry(0, 4, 10, 0).history(), Some(HistoryEntry::Reschedule { time: 100, interval_secs: 10 * 86_400 }));
        assert_eq!(entry(0, 5, 3, 0).history(), Some(HistoryEntry::Reschedule { time: 100, interval_secs: 3 * 86_400 }));
        assert_eq!(entry(3, 4, 3, 0).history(), None);
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
        // Anki's "type in the answer" note type shows the answer only in the typing box.
        assert_eq!(block("{{Front}}<hr id=answer>{{type:Back}}"), "Q: q\nA: a");
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
    fn layouts_follow_templates() {
        let notetype = |templates: Vec<(&str, &str)>| Notetype {
            name: "Vocab".into(),
            cloze: false,
            fields: ["Word", "Word Audio", "Meaning", "Hanja", "Sentence", "Sentence Audio"].map(String::from).to_vec(),
            templates: templates.into_iter().map(|(q, a)| ("Card".to_string(), q.to_string(), a.to_string())).collect(),
        };
        let rules = |templates| note_layout(&notetype(templates)).unwrap().toml().lines().skip(2).collect::<Vec<_>>().join("\n");
        // Audio the answer shows after a question's field comes first, not after Hanja.
        let kaishi = "{{Word}} {{Word Audio}}<div>{{Meaning}}</div>{{#Hanja}}{{Hanja}}{{/Hanja}}<hr>{{Sentence}}{{Sentence Audio}}";
        assert_eq!(rules(vec![("{{Word}}<hr>{{Sentence}}", kaishi)]), "  \"Word, Sentence -> Word Audio, Sentence Audio, Meaning, Hanja\",\n]");
        // A typing box makes its field typed in, first on the answer.
        assert_eq!(
            rules(vec![("{{Meaning}} {{type:nc:Word}}", "{{FrontSide}}<hr id=answer>{{Hanja}} {{type:nc:Word}}")]),
            "  \"Meaning => Word, Hanja\",\n]"
        );
    }

    #[test]
    fn template_fields() {
        let shown = |t, front| shown_fields(t, front).map(|names| names.join(" "));
        assert_eq!(shown("{{#A}}{{text:A}}{{/A}} {{type:B}} {{hint:C}} {{FrontSide}} {{tts en_US:D}} <b>{{E}}</b>", true).unwrap(), "A E");
        assert_eq!(shown("{{hint:C}}{{#A}}{{B}}{{/A}}<script>x = '{{D}}'</script>", false).unwrap(), "C B");
        // What the question shows depends on another field, a script or an attribute.
        assert_eq!(shown("{{#IsSentence}}{{Sentence}}{{/IsSentence}}", true), None);
        assert_eq!(shown("{{Word}}<script></script>", true), None);
        assert_eq!(shown("<img src=\"{{Picture}}\">", false), None);
        assert!(!has_own_text("<div class=\"word\">{{Word}}</div><br>{{Audio}} &mdash; <style>.word { color: red }</style>"));
        assert!(has_own_text("Plural of {{Word}}?"));
    }

    #[test]
    fn cloze_field_names() {
        assert_eq!(cloze_fields("{{cloze:Text}}<br>{{Extra}}{{cloze:Other}}"), ["Text", "Other"]);
    }
}
