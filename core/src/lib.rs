//! amgi: plain-text flash cards scheduled with FSRS.
//!
//! A collection is a directory of Markdown files; each file is a deck. Review
//! state lives in `.amgi.sqlite` at the collection root.

pub mod anki;
pub mod parser;
pub mod scheduler;
pub mod scripts;
pub mod search;
pub mod store;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub use parser::CardKind;
pub use scheduler::{Rating, ReviewState};
pub use scripts::Scripts;
use parser::ParsedCard;
use scheduler::{Memory, Scheduler};
use store::{CardState, LogEntry, LoggedReview, Store};

uniffi::setup_scaffolding!();

pub const DB_FILE: &str = ".amgi.sqlite";
/// Folder at the collection root that imported images are copied to.
pub const MEDIA_DIR: &str = "media";
const DAY_SECS: i64 = 86_400;
/// The scheduling day rolls over at 4am local time, so late-night study counts as the same day.
const DAY_ROLLOVER_SECS: i64 = 4 * 3600;
/// When nothing else is left, cards due within this window are shown early.
const LEARN_AHEAD_SECS: i64 = 20 * 60;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AmgiError {
    #[error("I/O error: {message}")]
    Io { message: String },
    #[error("database error: {message}")]
    Database { message: String },
    #[error("unknown card: {id}")]
    UnknownCard { id: String },
    #[error("scheduler error: {message}")]
    Scheduler { message: String },
    #[error("import failed: {message}")]
    Import { message: String },
    /// A card that can't be added, with a sentence saying why.
    #[error("{message}")]
    InvalidCard { message: String },
}

impl From<std::io::Error> for AmgiError {
    fn from(e: std::io::Error) -> Self {
        AmgiError::Io {
            message: e.to_string(),
        }
    }
}

impl From<fsrs::FSRSError> for AmgiError {
    fn from(e: fsrs::FSRSError) -> Self {
        AmgiError::Scheduler {
            message: e.to_string(),
        }
    }
}

impl From<rusqlite::Error> for AmgiError {
    fn from(e: rusqlite::Error) -> Self {
        AmgiError::Database {
            message: e.to_string(),
        }
    }
}

type Result<T, E = AmgiError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TagInfo {
    /// Without `#`, in the spelling first seen.
    pub name: String,
    /// Cards with this tag, in any spelling.
    pub cards: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct DeckInfo {
    /// File path relative to the collection root, without the `.md` extension, or a
    /// folder. A deck contains the decks below it: `lang` holds `lang.md` and the `lang` folder.
    pub name: String,
    /// The `name` from the deck's front matter, or the last part of `name`.
    pub title: String,
    pub total: u32,
    pub due: u32,
    /// Cards answered Again, waiting minutes to come back. Due once that time is up.
    pub learning: u32,
    /// New cards available today, capped by the daily new-card limit.
    pub new: u32,
    /// The note type most of the deck file's notes have, for adding notes to it;
    /// `None` without notes.
    pub note_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StudyCard {
    pub id: String,
    pub deck: String,
    /// Without `#`.
    pub tags: Vec<String>,
    /// Source file relative to the collection root.
    pub path: String,
    pub line: u32,
    pub kind: CardKind,
    /// Markdown.
    pub front: String,
    /// Markdown.
    pub back: String,
    pub is_new: bool,
    /// Seconds until the card is due again for Again, Hard, Good, Easy.
    pub intervals: Vec<i64>,
}

/// A card as listed when browsing a deck.
#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct BrowseCard {
    pub id: String,
    pub deck: String,
    /// Without `#`.
    pub tags: Vec<String>,
    /// Source file relative to the collection root.
    pub path: String,
    pub line: u32,
    pub kind: CardKind,
    /// Markdown.
    pub front: String,
    /// Markdown.
    pub back: String,
    /// Unix seconds; `None` for a new card.
    pub due: Option<i64>,
    pub reps: u32,
    pub lapses: u32,
    /// Tagged `#suspended`, so never studied.
    pub suspended: bool,
    /// The note type of the note the card comes from; `None` for `Q:` and `C:` cards.
    #[serde(rename = "noteType")]
    pub note_type: Option<String>,
    /// The note's fields as Markdown, in its note type's order; empty without a note.
    pub fields: Vec<String>,
}

/// Everything known about one card: its content, schedule, memory and review history.
#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct CardInfo {
    pub id: String,
    pub deck: String,
    /// Without `#`.
    pub tags: Vec<String>,
    /// Source file relative to the collection root.
    pub path: String,
    pub line: u32,
    pub kind: CardKind,
    /// Markdown.
    pub front: String,
    /// Markdown.
    pub back: String,
    pub suspended: bool,
    /// Unix seconds; `None` for a new card.
    pub due: Option<i64>,
    /// Unix seconds; `None` for a new card.
    pub last_review: Option<i64>,
    pub reps: u32,
    pub lapses: u32,
    /// Days until recall falls to 90%; `None` for a new card.
    pub stability: Option<f64>,
    /// From 1 (easiest) to 10 (hardest); `None` for a new card.
    pub difficulty: Option<f64>,
    /// Probability of recalling it now; `None` for a new card.
    pub retrievability: Option<f64>,
    /// Oldest first.
    pub reviews: Vec<CardReview>,
}

/// One review of a card, with the memory state and interval it led to.
#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct CardReview {
    /// Unix seconds.
    pub reviewed_at: i64,
    pub rating: Rating,
    /// Study days since the review before; 0 for the first.
    pub elapsed_days: f64,
    pub stability: f64,
    pub difficulty: f64,
    /// Seconds until the card was due again.
    pub interval_secs: i64,
    pub was_new: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct CollectionWarning {
    pub path: String,
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct TodayStats {
    pub reviewed: u32,
    pub new_studied: u32,
    /// New cards still available today across the collection, capped by the daily limit.
    pub new_available: u32,
}

/// A card to add to a deck's file.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NewCard {
    /// As in `DeckInfo::name`; its file is created if needed.
    pub deck: String,
    pub kind: CardKind,
    /// Markdown: the question, or a cloze's text.
    pub front: String,
    /// Markdown; ignored for a cloze.
    pub back: String,
    /// With or without `#`.
    pub tags: Vec<String>,
}

/// A note to add to a deck.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NewNote {
    /// As in `DeckInfo::name`; its file is created if needed.
    pub deck: String,
    /// As in `NoteTypeInfo::name`.
    pub note_type: String,
    /// Markdown for each of the note type's fields, in order; empty ones are left out.
    pub values: Vec<String>,
    /// With or without `#`.
    pub tags: Vec<String>,
}

/// Decks brought in from another app, with the review history it kept.
#[derive(Debug, Default)]
pub struct Import {
    /// Deck path relative to the root, without `.md`, and its Markdown.
    pub decks: Vec<(String, String)>,
    /// Note type name and its file's TOML, for the note types the decks name.
    pub note_types: Vec<(String, String)>,
    /// Card id and its history, oldest first.
    pub histories: Vec<(String, Vec<HistoryEntry>)>,
}

/// One event in a card's imported history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HistoryEntry {
    /// The card was studied at `time` (Unix seconds).
    Review {
        time: i64,
        rating: Rating,
        /// The card's stage, if the other app knows it. `Learning` on a card with no
        /// history yet makes it the first review.
        state: Option<ReviewState>,
        /// When the other app scheduled the card next; `None` lets the scheduler decide.
        interval_secs: Option<i64>,
    },
    /// The card was made new again.
    Reset { time: i64 },
    /// The card's due date was moved to `interval_secs` after `time`.
    Reschedule { time: i64, interval_secs: i64 },
}

/// What [`CardCollection::import`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// Deck files written, relative to the collection root.
    pub files: Vec<String>,
    /// Cards that kept their imported history.
    pub reviewed_cards: u32,
    /// Cards that already had review history in this collection, which was kept.
    pub existing_history: u32,
    /// Cards never reviewed, there or here.
    pub new_cards: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct Dashboard {
    /// Reviews per study day, oldest first, ending today.
    pub activity: Vec<u32>,
    /// Unix seconds at which the first day of `activity` starts.
    pub activity_start: i64,
    /// Cards due per day from today on; today includes overdue cards.
    pub forecast: Vec<u32>,
    /// Consecutive study days up to today, or up to yesterday if today has no reviews yet.
    pub streak: u32,
    pub new_cards: u32,
    /// Seen cards scheduled less than 21 days after their last review.
    pub young_cards: u32,
    pub mature_cards: u32,
    /// Tagged `#suspended`; not counted as new, young or mature.
    pub suspended_cards: u32,
    /// Share of reviews in the last 30 days, of cards last seen a day or more before,
    /// that were not rated Again. `None` without such reviews.
    pub retention: Option<f64>,
}

const MATURE_SECS: i64 = 21 * DAY_SECS;
const FORECAST_DAYS: usize = 14;

/// Which cards to study or browse. Tags and saved views are searches, run by the browse pane.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StudyScope {
    All,
    /// A deck and its subdecks.
    Deck { name: String },
}

impl StudyScope {
    fn contains(&self, card: &Card) -> bool {
        match self {
            StudyScope::All => true,
            StudyScope::Deck { name } => contains(name, &card.deck),
        }
    }
}

/// A named browse search, listed under Saved Views.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct SavedView {
    pub id: i64,
    pub name: String,
    /// In the browse search syntax, such as `is:leech`.
    pub query: String,
    /// Such as an emoji.
    pub icon: Option<String>,
}

struct Card {
    parsed: ParsedCard,
    deck: String,
    path: String,
}

impl Card {
    fn suspended(&self) -> bool {
        self.parsed.is_suspended()
    }
}

struct Inner {
    root: PathBuf,
    store: Store,
    scheduler: Scheduler,
    cards: Vec<Card>,
    index: HashMap<String, usize>,
    /// Deck → name from its front matter.
    deck_titles: HashMap<String, String>,
    /// The note type most of each deck file's notes have, for those with notes.
    deck_note_types: HashMap<String, String>,
    states: HashMap<String, CardState>,
    warnings: Vec<CollectionWarning>,
    /// From the `note-types` folder, read with the decks.
    note_types: parser::NoteTypes,
    /// CJK scripts in the deck files, for the fonts the app fetches.
    scripts: Scripts,
    new_per_day: u32,
    utc_offset_secs: i64,
    /// Why the database was reopened while the collection was open, shown with the
    /// warnings until the app restarts.
    store_notice: Option<CollectionWarning>,
    /// Answers given since the collection was opened, latest last, for `undo_answer`.
    answered: Vec<Answered>,
}

/// An answer that can be taken back: the card, its review's log row and its state before.
struct Answered {
    card_id: String,
    review_id: i64,
    prev: Option<CardState>,
}

#[derive(uniffi::Object)]
pub struct CardCollection {
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl CardCollection {
    /// Open the collection rooted at `root`, creating its database if needed.
    #[uniffi::constructor]
    pub fn open(root: String) -> Result<Arc<Self>> {
        let root = PathBuf::from(root);
        let store = Store::open(&root.join(DB_FILE))?;
        Self::with_store(root, store).map(Arc::new)
    }

    /// Re-read all Markdown files. Call after files change on disk.
    pub fn reload(&self) -> Result<()> {
        self.lock().reload()
    }

    pub fn set_new_cards_per_day(&self, limit: u32) {
        self.lock().new_per_day = limit;
    }

    /// Local time zone offset, used to decide when the study day rolls over.
    pub fn set_utc_offset(&self, seconds: i32) {
        self.lock().utc_offset_secs = seconds.into();
    }

    /// Target probability of recall when a card comes due, between 0.7 and 0.99.
    pub fn set_desired_retention(&self, retention: f32) {
        self.lock().scheduler.desired_retention = retention.clamp(0.7, 0.99);
    }

    /// Every deck and folder, parents before children. Counts include subdecks.
    pub fn decks(&self) -> Result<Vec<DeckInfo>> {
        self.lock().decks(unix_now())
    }

    pub fn next_card(&self, scope: StudyScope) -> Result<Option<StudyCard>> {
        self.lock().next_card(&scope, unix_now())
    }

    /// Every card in `scope`, in file order.
    pub fn cards(&self, scope: StudyScope) -> Vec<BrowseCard> {
        self.lock().browse(&scope)
    }

    /// `cards` as a JSON array, for the browse pane. Writing it here skips converting
    /// every card to Swift and back, which takes most of the time for large decks.
    pub fn cards_json(&self, scope: StudyScope) -> String {
        // Plain structs of strings and numbers always serialize.
        serde_json::to_string(&self.lock().browse(&scope)).expect("cards serialize")
    }

    /// The collection's note types as a JSON array, each with its `name` and `fields`,
    /// for the browse pane.
    pub fn note_types_json(&self) -> String {
        serde_json::to_string(&self.note_types()).expect("note types serialize")
    }

    /// The cards with these ids, in file order. Unknown ids are skipped.
    pub fn cards_with_ids(&self, ids: Vec<String>) -> Vec<BrowseCard> {
        let ids: HashSet<String> = ids.into_iter().collect();
        let inner = self.lock();
        inner.browse_where(&StudyScope::All, |card| ids.contains(&card.parsed.id))
    }

    /// Every tag in the collection, sorted, in the spelling first seen.
    pub fn tags(&self) -> Vec<TagInfo> {
        self.lock().tags()
    }

    /// Icons chosen for tags, such as an emoji, keyed by lowercased tag.
    pub fn tag_icons(&self) -> Result<HashMap<String, String>> {
        Ok(self.lock().store.tag_icons()?)
    }

    /// Set or, with `None` or an empty string, remove a tag's icon. Tags compare case-insensitively.
    pub fn set_tag_icon(&self, tag: String, icon: Option<String>) -> Result<()> {
        let icon = icon.as_deref().map(str::trim).filter(|i| !i.is_empty());
        Ok(self.lock().store.set_tag_icon(&tag, icon)?)
    }

    /// Saved views in the order they were added.
    pub fn saved_views(&self) -> Result<Vec<SavedView>> {
        Ok(self.lock().store.saved_views()?)
    }

    /// Adds a view and returns its id.
    pub fn add_saved_view(&self, name: String, query: String) -> Result<i64> {
        Ok(self.lock().store.add_saved_view(name.trim(), query.trim())?)
    }

    /// Changes the name, search and icon of the view with `view.id`. An empty icon removes it.
    pub fn update_saved_view(&self, view: SavedView) -> Result<()> {
        let view = SavedView {
            name: view.name.trim().to_string(),
            query: view.query.trim().to_string(),
            icon: view.icon.map(|i| i.trim().to_string()).filter(|i| !i.is_empty()),
            ..view
        };
        Ok(self.lock().store.update_saved_view(&view)?)
    }

    pub fn delete_saved_view(&self, id: i64) -> Result<()> {
        Ok(self.lock().store.delete_saved_view(id)?)
    }

    pub fn answer(&self, card_id: String, rating: Rating) -> Result<()> {
        self.lock().answer(&card_id, rating, unix_now())
    }

    /// Take back the latest answer not yet undone, removing it from the history.
    /// Returns its card, scheduled as before, or `None` if there is nothing to undo or
    /// the card has since left the collection.
    pub fn undo_answer(&self) -> Result<Option<StudyCard>> {
        self.lock().undo_answer(unix_now())
    }

    /// Whether `undo_answer` has an answer to take back.
    pub fn can_undo_answer(&self) -> bool {
        !self.lock().answered.is_empty()
    }

    /// `None` for a card no longer in the collection.
    pub fn card_info(&self, card_id: String) -> Result<Option<CardInfo>> {
        self.lock().card_info(&card_id, unix_now())
    }

    /// Suspend or unsuspend cards by adding or removing `#suspended` on their tag
    /// lines, then reload. A cloze is one card in its file, so all its deletions
    /// change together. Returns how many cards in the files changed; one suspended by
    /// its deck's front matter cannot be unsuspended here.
    pub fn set_suspended(&self, card_ids: Vec<String>, suspended: bool) -> Result<u32> {
        self.lock().set_suspended(&card_ids, suspended)
    }

    /// Append a card to the end of its deck's file. Returns the ids of the cards it
    /// makes: one for a basic card, one per gap for a cloze.
    pub fn add_card(&self, card: NewCard) -> Result<Vec<String>> {
        self.lock().add_card(&card)
    }

    /// Append a note to the end of its deck's file. Returns the ids of the cards it makes.
    pub fn add_note(&self, note: NewNote) -> Result<Vec<String>> {
        self.lock().add_note(&note)
    }

    /// How many cards a note of `note_type` would make: 0 if it can't be added as it is.
    pub fn note_card_count(&self, note_type: String, values: Vec<String>) -> u32 {
        parser::write_note(&note_type, &self.lock().note_types, &values, &[]).map_or(0, |(_, ids)| ids.len() as u32)
    }

    /// The note types without errors, by name.
    pub fn note_types(&self) -> Vec<parser::NoteTypeInfo> {
        self.lock().note_types.list()
    }

    /// Copy an image or sound into the `media` folder for a card in `deck`, and return
    /// the Markdown that shows it there. A file with the same name and contents is
    /// shared; one that differs keeps its name, and the copy gets another.
    pub fn add_media(&self, deck: String, path: String) -> Result<String> {
        let root = self.root();
        let source = PathBuf::from(&path);
        let bytes = fs::read(&source)?;
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "media".into());
        let (stem, ext) = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
            _ => (name.clone(), String::new()),
        };
        let dir = root.join(MEDIA_DIR);
        fs::create_dir_all(&dir)?;
        let mut file = name.clone();
        for n in 2.. {
            match fs::read(dir.join(&file)) {
                Ok(existing) if existing == bytes => break,
                Ok(_) => file = format!("{stem} {n}{ext}"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::write(dir.join(&file), &bytes)?;
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let link = format!("{}{MEDIA_DIR}/{file}", "../".repeat(deck.matches('/').count()));
        Ok(if link.contains([' ', '(', ')', '<', '>']) { format!("![](<{link}>)") } else { format!("![]({link})") })
    }

    pub fn today(&self) -> Result<TodayStats> {
        self.lock().today(unix_now())
    }

    /// Collection-wide statistics, with `activity_days` days of review history.
    pub fn dashboard(&self, activity_days: u32) -> Result<Dashboard> {
        self.lock().dashboard(activity_days, unix_now())
    }

    pub fn warnings(&self) -> Vec<CollectionWarning> {
        let inner = self.lock();
        inner.warnings.iter().chain(&inner.store_notice).cloned().collect()
    }

    /// Whether the deck files use Hangul, kana or Han characters, as of the last read.
    pub fn scripts(&self) -> Scripts {
        self.lock().scripts
    }
}

impl CardCollection {
    /// The folder holding the collection's decks.
    pub fn root(&self) -> PathBuf {
        self.lock().root.clone()
    }

    /// Write decks from another app and replay their review history. Existing files are
    /// never overwritten, and history already here is kept.
    pub fn import(&self, import: Import) -> Result<Imported> {
        self.lock().import(import)
    }

    fn with_store(root: PathBuf, store: Store) -> Result<Self> {
        let states = store.states()?;
        let mut inner = Inner {
            root,
            store,
            scheduler: Scheduler::default(),
            cards: Vec::new(),
            index: HashMap::new(),
            deck_titles: HashMap::new(),
            deck_note_types: HashMap::new(),
            states,
            warnings: Vec::new(),
            note_types: parser::NoteTypes::default(),
            scripts: Scripts::default(),
            new_per_day: 20,
            utc_offset_secs: 0,
            store_notice: None,
            answered: Vec::new(),
        };
        inner.reload()?;
        Ok(CardCollection {
            inner: Mutex::new(inner),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic mid-review leaves no partial state worth discarding: SQLite writes are transactional.
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.reopen_replaced_store();
        inner
    }
}

impl Inner {
    /// If the database file was deleted or replaced while open, switch to whatever is
    /// at its path now, as a restart would: a new, empty database if it was deleted.
    /// Otherwise writes would go to a file that is gone.
    fn reopen_replaced_store(&mut self) {
        let Some(path) = self.store.replaced().map(Path::to_path_buf) else { return };
        let deleted = !path.exists();
        let reopened = Store::open(&path).and_then(|store| Ok((store.states()?, store)));
        let message = match reopened {
            Ok((states, store)) => {
                self.store = store;
                self.states = states;
                // Those reviews were in the old file.
                self.answered.clear();
                if deleted {
                    "The review database was deleted, so a new one was started.".to_string()
                } else {
                    "The review database was replaced on disk, so it was opened again.".to_string()
                }
            }
            Err(e) => format!("The review database was deleted or replaced and couldn't be opened again: {e}"),
        };
        self.store_notice = Some(CollectionWarning { path: relative(&self.root, &path), line: 0, message });
    }

    fn reload(&mut self) -> Result<()> {
        let mut files = Vec::new();
        collect_markdown(&self.root, &mut files)?;
        // By deck name, so a deck's own cards come before its subdecks'.
        files.sort_by_key(|f| f.with_extension(""));

        self.cards.clear();
        self.index.clear();
        self.deck_titles.clear();
        self.deck_note_types.clear();
        self.warnings.clear();
        self.scripts = Scripts::default();
        self.load_note_types()?;
        for file in files {
            let rel = relative(&self.root, &file);
            let deck = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
            let text = fs::read_to_string(&file)?;
            self.scripts.scan(&text);
            let result = parser::parse_deck(&text, &self.note_types);
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for note in result.cards.iter().filter_map(|c| c.note.as_ref()) {
                *counts.entry(&note.note_type).or_default() += 1;
            }
            if let Some((name, _)) = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))) {
                self.deck_note_types.insert(deck.clone(), name.to_string());
            }
            if let Some(name) = result.name {
                self.deck_titles.insert(deck.clone(), name);
            }
            for w in result.warnings {
                self.warnings.push(CollectionWarning {
                    path: rel.clone(),
                    line: w.line,
                    message: w.message,
                });
            }
            for parsed in result.cards {
                if let Some(&i) = self.index.get(&parsed.id) {
                    let first = &self.cards[i];
                    self.warnings.push(CollectionWarning {
                        path: rel.clone(),
                        line: parsed.line,
                        message: format!("duplicate of {}:{}", first.path, first.parsed.line),
                    });
                    continue;
                }
                self.index.insert(parsed.id.clone(), self.cards.len());
                self.cards.push(Card {
                    parsed,
                    deck: deck.clone(),
                    path: rel.clone(),
                });
            }
        }
        Ok(())
    }

    /// Every `note-types/Name.toml`, as note type `Name`.
    fn load_note_types(&mut self) -> Result<()> {
        self.note_types = parser::NoteTypes::default();
        let dir = self.root.join(parser::NOTE_TYPES_DIR);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml") && p.is_file())
            .collect();
        files.sort();
        for file in files {
            let Some(name) = file.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            if name.starts_with('.') {
                continue;
            }
            for w in self.note_types.insert(&name, &fs::read_to_string(&file)?) {
                self.warnings.push(CollectionWarning { path: relative(&self.root, &file), line: w.line, message: w.message });
            }
        }
        Ok(())
    }

    fn day_start(&self, now: i64) -> i64 {
        let local = now + self.utc_offset_secs - DAY_ROLLOVER_SECS;
        local.div_euclid(DAY_SECS) * DAY_SECS + DAY_ROLLOVER_SECS - self.utc_offset_secs
    }

    fn new_remaining(&self, now: i64) -> Result<u32> {
        let studied = self.store.new_cards_since(self.day_start(now))?;
        Ok(self.new_per_day.saturating_sub(studied))
    }

    fn in_scope<'a, 's>(&'a self, scope: &'s StudyScope) -> impl Iterator<Item = &'a Card> + use<'a, 's> {
        self.cards.iter().filter(move |c| scope.contains(c))
    }

    fn tags(&self) -> Vec<TagInfo> {
        // Lowercased tag → its index in `tags`.
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut tags: Vec<TagInfo> = Vec::new();
        for card in &self.cards {
            // A card counts once per tag, however many spellings it has.
            let mut counted = HashSet::new();
            for tag in &card.parsed.tags {
                let key = tag.to_lowercase();
                let i = *index.entry(key.clone()).or_insert_with(|| {
                    tags.push(TagInfo { name: tag.clone(), cards: 0 });
                    tags.len() - 1
                });
                if counted.insert(key) {
                    tags[i].cards += 1;
                }
            }
        }
        tags.sort_by_key(|t| t.name.to_lowercase());
        tags
    }

    fn decks(&self, now: i64) -> Result<Vec<DeckInfo>> {
        let remaining = self.new_remaining(now)?;
        // Count each deck's own cards, then add them to the deck and its parents:
        // there are far fewer decks than cards.
        let mut own: HashMap<&str, (u32, u32, u32, u32)> = HashMap::new();
        for card in &self.cards {
            let counts = own.entry(&card.deck).or_default();
            counts.0 += 1;
            if card.suspended() {
                continue;
            }
            match self.states.get(&card.parsed.id) {
                Some(s) if s.due <= now => counts.1 += 1,
                Some(s) if s.interval_secs < DAY_SECS => counts.2 += 1,
                Some(_) => {}
                None => counts.3 += 1,
            }
        }
        // Keyed by path components, so parents sort before their children.
        let mut decks: BTreeMap<Vec<&str>, DeckInfo> = BTreeMap::new();
        for (name, (total, due, learning, new)) in own {
            let parts: Vec<&str> = name.split('/').collect();
            for depth in 1..=parts.len() {
                let deck = decks.entry(parts[..depth].to_vec()).or_insert_with(|| DeckInfo {
                    name: parts[..depth].join("/"),
                    title: String::new(),
                    total: 0,
                    due: 0,
                    learning: 0,
                    new: 0,
                    note_type: None,
                });
                deck.total += total;
                deck.due += due;
                deck.learning += learning;
                deck.new += new;
            }
        }
        Ok(decks
            .into_values()
            .map(|d| DeckInfo {
                title: self.deck_titles.get(&d.name).cloned().unwrap_or_else(|| {
                    d.name.rsplit('/').next().unwrap_or_default().to_string()
                }),
                new: d.new.min(remaining),
                note_type: self.deck_note_types.get(&d.name).cloned(),
                ..d
            })
            .collect())
    }

    fn browse(&self, scope: &StudyScope) -> Vec<BrowseCard> {
        self.browse_where(scope, |_| true)
    }

    fn browse_where(&self, scope: &StudyScope, keep: impl Fn(&Card) -> bool) -> Vec<BrowseCard> {
        self.in_scope(scope)
            .filter(|card| keep(card))
            .map(|card| {
                let state = self.states.get(&card.parsed.id);
                BrowseCard {
                    id: card.parsed.id.clone(),
                    deck: card.deck.clone(),
                    tags: card.parsed.tags.clone(),
                    path: card.path.clone(),
                    line: card.parsed.line,
                    kind: card.parsed.kind,
                    front: card.parsed.front.clone(),
                    back: card.parsed.back.clone(),
                    due: state.map(|s| s.due),
                    reps: state.map_or(0, |s| s.reps),
                    lapses: state.map_or(0, |s| s.lapses),
                    suspended: card.suspended(),
                    note_type: card.parsed.note.as_ref().map(|n| n.note_type.clone()),
                    fields: card.parsed.note.as_ref().map(|n| n.values.clone()).unwrap_or_default(),
                }
            })
            .collect()
    }

    fn next_card(&self, scope: &StudyScope, now: i64) -> Result<Option<StudyCard>> {
        let studied = || self.in_scope(scope).filter(|c| !c.suspended());
        let seen = || studied().filter_map(|c| self.states.get(&c.parsed.id).map(|s| (s, c)));
        let earliest_due_before = |limit: i64| {
            seen().filter(|(s, _)| s.due <= limit).min_by_key(|(s, _)| s.due).map(|(_, c)| c)
        };
        // A failed card comes back within minutes, ahead of reviews, while the
        // lapse is fresh. Its due date alone can't tell: a review due tomorrow
        // starts at the day's rollover, which can be just hours away.
        let relearning = seen()
            .filter(|(s, _)| s.due <= now && s.interval_secs < DAY_SECS)
            .min_by_key(|(s, _)| s.due)
            .map(|(_, c)| c);
        // Reviews, most overdue day first. Within a day a fixed shuffle mixes decks,
        // and keeps its order when the app restarts.
        let review = || {
            seen()
                .filter(|(s, _)| s.due <= now)
                .min_by_key(|(s, c)| {
                    let last_review = s.last_review.to_string();
                    (self.day_start(s.due), parser::stable_hash(&[&c.parsed.id, &last_review]))
                })
                .map(|(_, c)| c)
        };

        let card = match relearning.or_else(review) {
            Some(c) => Some(c),
            None if self.new_remaining(now)? > 0 => studied()
                .find(|c| !self.states.contains_key(&c.parsed.id))
                .or_else(|| earliest_due_before(now + LEARN_AHEAD_SECS)),
            None => earliest_due_before(now + LEARN_AHEAD_SECS),
        };
        card.map(|c| self.study_card(c, now)).transpose()
    }

    /// Memory state and whole study days since the last review (0 on the same day).
    fn elapsed(&self, state: Option<&CardState>, now: i64) -> (Option<Memory>, u32) {
        match state {
            Some(s) => {
                let days = (self.day_start(now) - self.day_start(s.last_review)) / DAY_SECS;
                (Some(s.memory), days.max(0) as u32)
            }
            None => (None, 0),
        }
    }

    fn study_card(&self, card: &Card, now: i64) -> Result<StudyCard> {
        let state = self.states.get(&card.parsed.id);
        let (memory, elapsed) = self.elapsed(state, now);
        let preview = self.scheduler.preview(memory, elapsed)?;
        Ok(StudyCard {
            id: card.parsed.id.clone(),
            deck: card.deck.clone(),
            tags: card.parsed.tags.clone(),
            path: card.path.clone(),
            line: card.parsed.line,
            kind: card.parsed.kind,
            front: card.parsed.front.clone(),
            back: card.parsed.back.clone(),
            is_new: state.is_none(),
            intervals: preview.iter().map(|o| o.interval_secs).collect(),
        })
    }

    fn card_info(&self, card_id: &str, now: i64) -> Result<Option<CardInfo>> {
        let Some(&i) = self.index.get(card_id) else { return Ok(None) };
        let card = &self.cards[i];
        let state = self.states.get(card_id);
        let reviews = self.store.reviews(card_id)?;
        Ok(Some(CardInfo {
            id: card.parsed.id.clone(),
            deck: card.deck.clone(),
            tags: card.parsed.tags.clone(),
            path: card.path.clone(),
            line: card.parsed.line,
            kind: card.parsed.kind,
            front: card.parsed.front.clone(),
            back: card.parsed.back.clone(),
            suspended: card.suspended(),
            due: state.map(|s| s.due),
            last_review: state.map(|s| s.last_review),
            reps: state.map_or(0, |s| s.reps),
            lapses: state.map_or(0, |s| s.lapses),
            stability: state.map(|s| s.memory.stability),
            difficulty: state.map(|s| s.memory.difficulty),
            retrievability: state.map(|s| {
                let days = (now - s.last_review) as f64 / DAY_SECS as f64;
                self.scheduler.retrievability(s.memory, days)
            }),
            reviews: reviews
                .into_iter()
                .map(|r| CardReview {
                    reviewed_at: r.reviewed_at,
                    rating: r.rating,
                    elapsed_days: r.elapsed_days,
                    stability: r.memory.stability,
                    difficulty: r.memory.difficulty,
                    interval_secs: r.interval_secs,
                    was_new: r.was_new,
                })
                .collect(),
        }))
    }

    fn answer(&mut self, card_id: &str, rating: Rating, now: i64) -> Result<()> {
        if !self.index.contains_key(card_id) {
            return Err(AmgiError::UnknownCard { id: card_id.into() });
        }
        let prev = self.states.get(card_id).copied();
        let (memory, elapsed) = self.elapsed(prev.as_ref(), now);
        let outcome = self.scheduler.schedule(memory, elapsed, rating)?;
        // Reviews logged before `review_state` existed count as plain reviews.
        let review_state = match prev {
            None => ReviewState::New,
            Some(_) => ReviewState::after(
                self.store.last_review_state(card_id)?.unwrap_or(ReviewState::Review),
                elapsed,
            ),
        };
        let (state, review_id) = self
            .store
            .record_review(card_id, now, rating, elapsed.into(), review_state, prev, outcome)?;
        self.states.insert(card_id.to_string(), state);
        self.answered.push(Answered { card_id: card_id.to_string(), review_id, prev });
        Ok(())
    }

    fn undo_answer(&mut self, now: i64) -> Result<Option<StudyCard>> {
        let Some(answered) = self.answered.pop() else { return Ok(None) };
        let Answered { card_id, review_id, prev } = answered;
        if let Err(e) = self.store.undo_review(&card_id, review_id, prev) {
            self.answered.push(Answered { card_id, review_id, prev });
            return Err(e.into());
        }
        match prev {
            Some(state) => self.states.insert(card_id.clone(), state),
            None => self.states.remove(&card_id),
        };
        match self.index.get(&card_id) {
            Some(&i) => self.study_card(&self.cards[i], now).map(Some),
            None => Ok(None),
        }
    }

    fn add_card(&mut self, card: &NewCard) -> Result<Vec<String>> {
        let invalid = |message: &str| AmgiError::InvalidCard { message: message.into() };
        let block = parser::write_card(card.kind, &card.front, &card.back, &card.tags)
            .map_err(|message| AmgiError::InvalidCard { message })?;
        let deck = card.deck.trim().trim_matches('/');
        if deck.is_empty() || deck.split('/').any(|part| part.trim().is_empty() || part.starts_with('.')) {
            return Err(invalid("Choose a deck for the card."));
        }
        let ids: Vec<String> = parser::parse(&block).cards.into_iter().map(|c| c.id).collect();
        if let Some(existing) = ids.iter().find_map(|id| self.index.get(id)) {
            let path = &self.cards[*existing].path;
            return Err(AmgiError::InvalidCard { message: format!("This card is already in {path}.") });
        }

        self.append(deck, &block, &ids)?;
        Ok(ids)
    }

    fn add_note(&mut self, note: &NewNote) -> Result<Vec<String>> {
        let deck = note.deck.trim().trim_matches('/');
        if deck.is_empty() || deck.split('/').any(|part| part.trim().is_empty() || part.starts_with('.')) {
            return Err(AmgiError::InvalidCard { message: "Choose a deck for the note.".into() });
        }
        let (block, ids) = parser::write_note(&note.note_type, &self.note_types, &note.values, &note.tags)
            .map_err(|message| AmgiError::InvalidCard { message })?;
        if let Some(existing) = ids.iter().find_map(|id| self.index.get(id)) {
            let card = &self.cards[*existing];
            return Err(AmgiError::InvalidCard {
                message: format!("A card with the same front is already in {}:{}.", card.path, card.parsed.line),
            });
        }
        self.append(deck, &block, &ids)?;
        Ok(ids)
    }

    /// Append `block` to the end of `deck`'s file, which is created if needed, and
    /// check that it makes the cards `ids`; if not, the file is put back.
    fn append(&mut self, deck: &str, block: &str, ids: &[String]) -> Result<()> {
        let path = self.root.join(format!("{deck}.md"));
        let before = match fs::read_to_string(&path) {
            Ok(source) => Some(source),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let mut source = before.clone().unwrap_or_default();
        if !source.trim().is_empty() {
            // A blank line ends the card before it.
            let end = source.trim_end().len();
            source.truncate(end);
            source.push_str("\n\n");
        }
        source.push_str(block);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &source)?;
        self.reload()?;

        // The end of the file can swallow the card, as an unclosed code block would.
        if ids.iter().all(|id| self.index.contains_key(id)) {
            return Ok(());
        }
        match before {
            Some(before) => fs::write(&path, before)?,
            None => fs::remove_file(&path)?,
        }
        self.reload()?;
        Err(AmgiError::InvalidCard {
            message: format!("The end of {deck}.md would make the card part of something else, such as an unclosed code block."),
        })
    }

    fn set_suspended(&mut self, ids: &[String], suspended: bool) -> Result<u32> {
        // Each card's block once: a cloze's deletions share one.
        let mut blocks: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
        for id in ids {
            let card = &self.cards[*self.index.get(id).ok_or_else(|| AmgiError::UnknownCard { id: id.clone() })?];
            blocks.entry(&card.path).or_default().insert(card.parsed.line);
        }
        let mut changed = 0;
        for (path, lines) in blocks {
            let file = self.root.join(path);
            let mut source = fs::read_to_string(&file)?;
            let before = changed;
            // From the bottom up, so an added line never moves a card still to edit.
            for &line in lines.iter().rev() {
                if let Some(edited) = parser::set_card_tag(&source, &self.note_types, line, parser::SUSPENDED_TAG, suspended) {
                    source = edited;
                    changed += 1;
                }
            }
            if changed > before {
                fs::write(&file, source)?;
            }
        }
        self.reload()?;
        Ok(changed)
    }

    fn dashboard(&self, activity_days: u32, now: i64) -> Result<Dashboard> {
        // Day `n` starts at `n * DAY_SECS - shift`, matching `day_start`.
        let shift = self.utc_offset_secs - DAY_ROLLOVER_SECS;
        let today = (now + shift).div_euclid(DAY_SECS);
        let first = today - i64::from(activity_days.max(1)) + 1;

        // Every day studied, for the streak; the recent ones fill the activity heatmap.
        let per_day = self.store.reviews_per_day(0, shift)?;
        let mut activity = vec![0; activity_days.max(1) as usize];
        for &(day, count) in &per_day {
            if let Some(slot) = usize::try_from(day - first).ok().and_then(|i| activity.get_mut(i)) {
                *slot = count;
            }
        }

        let studied: HashSet<i64> = per_day.into_iter().map(|(d, _)| d).collect();
        let from = if studied.contains(&today) { today } else { today - 1 };
        let streak = (0..).take_while(|n| studied.contains(&(from - n))).count() as u32;

        let mut forecast = vec![0; FORECAST_DAYS];
        let (mut new_cards, mut young_cards, mut mature_cards, mut suspended_cards) = (0, 0, 0, 0);
        for card in &self.cards {
            if card.suspended() {
                suspended_cards += 1;
                continue;
            }
            let Some(state) = self.states.get(&card.parsed.id) else {
                new_cards += 1;
                continue;
            };
            if state.due - state.last_review >= MATURE_SECS {
                mature_cards += 1;
            } else {
                young_cards += 1;
            }
            let day = ((state.due + shift).div_euclid(DAY_SECS) - today).max(0);
            if let Some(slot) = forecast.get_mut(day as usize) {
                *slot += 1;
            }
        }

        let (recalled_of, recalled) = self.store.recall_since(self.day_start(now) - 29 * DAY_SECS)?;
        Ok(Dashboard {
            activity,
            activity_start: first * DAY_SECS - shift,
            forecast,
            streak,
            new_cards,
            young_cards,
            mature_cards,
            suspended_cards,
            retention: (recalled_of > 0).then(|| f64::from(recalled) / f64::from(recalled_of)),
        })
    }

    fn import(&mut self, import: Import) -> Result<Imported> {
        let mut files = Vec::new();
        // A note type already here with the same definition is shared; a different one
        // keeps its file, and the new one takes the next free name.
        let mut renamed: Vec<(String, String)> = Vec::new();
        let dir = self.root.join(parser::NOTE_TYPES_DIR);
        for (name, toml) in &import.note_types {
            for n in 1.. {
                let candidate = if n == 1 { name.clone() } else { format!("{name} {n}") };
                let path = dir.join(format!("{candidate}.toml"));
                match fs::read_to_string(&path) {
                    Ok(existing) if existing == *toml => {}
                    Ok(_) => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir_all(&dir)?;
                        fs::write(&path, toml)?;
                        files.push(relative(&self.root, &path));
                    }
                    Err(e) => return Err(e.into()),
                }
                if candidate != *name {
                    renamed.push((name.clone(), candidate));
                }
                break;
            }
        }
        for (deck, markdown) in &import.decks {
            let mut markdown = markdown.clone();
            if !renamed.is_empty() {
                let lines: Vec<String> = markdown
                    .split_inclusive('\n')
                    .map(|line| match renamed.iter().find(|(old, _)| line.trim_end() == format!("@{old}")) {
                        Some((_, new)) => format!("@{new}\n"),
                        None => line.to_string(),
                    })
                    .collect();
                markdown = lines.concat();
            }
            let path = unused_path(&self.root, deck);
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&path, markdown)?;
            files.push(relative(&self.root, &path));
        }

        // History never replaces what was already studied in amgi.
        let mut seen = HashSet::new();
        let mut imported = Vec::new();
        for (id, log) in &import.histories {
            if self.states.contains_key(id) || !seen.insert(id) {
                continue;
            }
            if let Some((reviews, state)) = self.replay(log)? {
                imported.push((id.clone(), reviews, state));
            }
        }
        self.store.import(&imported)?;
        let reviewed_cards = imported.len() as u32;
        self.states.extend(imported.into_iter().map(|(id, _, state)| (id, state)));
        self.reload()?;

        // Every card written, once each: a package can hold the same card twice.
        let written: HashSet<String> = import
            .decks
            .iter()
            .flat_map(|(_, markdown)| parser::parse_deck(markdown, &self.note_types).cards)
            .map(|card| card.id)
            .collect();
        let studied = written.iter().filter(|id| self.states.contains_key(*id)).count() as u32;

        Ok(Imported {
            files,
            reviewed_cards,
            existing_history: studied.saturating_sub(reviewed_cards),
            new_cards: written.len() as u32 - studied,
        })
    }

    /// Rebuild a card's memory state by replaying its imported history through the
    /// scheduler. Due dates follow the other app's intervals so its schedule is kept.
    /// `None` if the card ends up new.
    fn replay(&self, log: &[HistoryEntry]) -> Result<Option<(Vec<LogEntry>, CardState)>> {
        let mut entries = Vec::new();
        let mut state: Option<CardState> = None;
        let mut last_review_state = ReviewState::New;
        for entry in log {
            match *entry {
                HistoryEntry::Review {
                    time,
                    rating,
                    state: stage,
                    interval_secs,
                } => {
                    let (memory, elapsed) = self.elapsed(state.as_ref(), time);
                    // A history that starts mid-way has no New review, which tells the
                    // optimizer to leave it out.
                    let review_state = match (stage, state) {
                        (Some(ReviewState::New | ReviewState::Learning), None) => ReviewState::New,
                        (Some(stage), _) => stage,
                        (None, None) => ReviewState::Review,
                        (None, Some(_)) => ReviewState::after(last_review_state, elapsed),
                    };
                    last_review_state = review_state;
                    let outcome = self.scheduler.schedule(memory, elapsed, rating)?;
                    let interval_secs = interval_secs.unwrap_or(outcome.interval_secs);
                    entries.push(LogEntry::Review(LoggedReview {
                        reviewed_at: time,
                        rating,
                        elapsed_days: elapsed.into(),
                        memory: outcome.memory,
                        interval_secs,
                        was_new: state.is_none(),
                        state: Some(review_state),
                    }));
                    let lapse = u32::from(state.is_some() && rating == Rating::Again);
                    state = Some(CardState {
                        memory: outcome.memory,
                        due: time + interval_secs,
                        last_review: time,
                        reps: state.map_or(0, |s| s.reps) + 1,
                        lapses: state.map_or(0, |s| s.lapses) + lapse,
                        interval_secs,
                    });
                }
                HistoryEntry::Reset { time } => {
                    if state.take().is_some() {
                        entries.push(LogEntry::Reset(time));
                    }
                }
                HistoryEntry::Reschedule { time, interval_secs } => {
                    if let Some(s) = state.as_mut() {
                        s.due = time + interval_secs;
                    }
                }
            }
        }
        Ok(state.map(|s| (entries, s)))
    }

    fn today(&self, now: i64) -> Result<TodayStats> {
        let since = self.day_start(now);
        let unseen = self
            .cards
            .iter()
            .filter(|c| !c.suspended() && !self.states.contains_key(&c.parsed.id))
            .count() as u32;
        Ok(TodayStats {
            reviewed: self.store.reviews_since(since)?,
            new_studied: self.store.new_cards_since(since)?,
            new_available: unseen.min(self.new_remaining(now)?),
        })
    }
}

/// Short human-readable interval for answer buttons: `10m`, `3d`, `1.5mo`, `2.1y`.
/// Probability of recalling a card with this stability (in days) `elapsed_days` after
/// its last review, for drawing its forgetting curve.
#[uniffi::export]
pub fn recall_probability(stability: f64, elapsed_days: f64) -> f64 {
    scheduler::retrievability(stability, elapsed_days)
}

#[uniffi::export]
pub fn format_interval(seconds: i64) -> String {
    let s = seconds.max(0) as f64;
    let (value, unit) = match s {
        s if s < 3600.0 => ((s / 60.0).max(1.0), "m"),
        s if s < 86_400.0 => (s / 3600.0, "h"),
        s if s < 30.0 * 86_400.0 => (s / 86_400.0, "d"),
        s if s < 365.0 * 86_400.0 => (s / (30.0 * 86_400.0), "mo"),
        s => (s / (365.0 * 86_400.0), "y"),
    };
    if value < 10.0 && unit != "m" && unit != "d" && value.fract() >= 0.05 {
        format!("{value:.1}{unit}")
    } else {
        format!("{}{unit}", value.round() as i64)
    }
}

/// Deck files under `dir`, leaving out hidden ones and the root's note types.
fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    collect_deck_files(dir, true, out)
}

fn collect_deck_files(dir: &Path, root: bool, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || (root && name == parser::NOTE_TYPES_DIR) {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_deck_files(&path, false, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether `deck` is `parent` or one of its subdecks.
fn contains(parent: &str, deck: &str) -> bool {
    deck.strip_prefix(parent).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// `root/deck.md`, or `root/deck 2.md` and so on if that file exists.
fn unused_path(root: &Path, deck: &str) -> PathBuf {
    (1..)
        .map(|n| match n {
            1 => root.join(format!("{deck}.md")),
            n => root.join(format!("{deck} {n}.md")),
        })
        .find(|p| !p.exists())
        .expect("unbounded range")
}

fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_760_000_000;

    fn deck(name: &str) -> StudyScope {
        StudyScope::Deck { name: name.into() }
    }

    fn collection(files: &[(&str, &str)]) -> (PathBuf, CardCollection) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("amgi-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (name, body) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        let c = CardCollection::with_store(dir.clone(), Store::open_in_memory().unwrap()).unwrap();
        (dir, c)
    }

    #[test]
    fn study_flow() {
        let (dir, c) = collection(&[
            ("a.md", "Q: 1\nA: one\n\nQ: 2\nA: two\n"),
            ("sub/b.md", "C: [x] and [y]\n\nQ: 1\nA: dup\n"),
            (".hidden/c.md", "Q: hidden\nA: no\n"),
        ]);
        let mut inner = c.lock();
        assert_eq!(inner.cards.len(), 4);
        assert_eq!(inner.warnings.len(), 1);
        assert!(inner.warnings[0].message.contains("duplicate of a.md:1"));

        let decks = inner.decks(T0).unwrap();
        let names: Vec<_> = decks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["a", "sub", "sub/b"]);
        assert_eq!((decks[1].total, decks[1].new), (2, 2));
        assert_eq!((decks[0].total, decks[0].new, decks[0].due), (2, 2, 0));

        // New cards come in file order.
        let card = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        assert_eq!(card.front, "1");
        assert!(card.is_new);
        assert_eq!(card.intervals.len(), 4);
        inner.answer(&card.id, Rating::Good, T0).unwrap();

        let card2 = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        assert_eq!(card2.front, "2");
        inner.answer(&card2.id, Rating::Again, T0).unwrap();

        // The failed card waits to come back, then is due.
        let decks = inner.decks(T0 + 60).unwrap();
        assert_eq!((decks[0].due, decks[0].learning, decks[0].new), (0, 1, 0));
        let decks = inner.decks(T0 + scheduler::AGAIN_DELAY_SECS).unwrap();
        assert_eq!((decks[0].due, decks[0].learning), (1, 0));

        // It is shown again within the learn-ahead window.
        let again = inner.next_card(&deck("a"), T0 + 60).unwrap().unwrap();
        assert_eq!(again.id, card2.id);
        assert!(!again.is_new);

        // Two days later the first card is due.
        let decks = inner.decks(T0 + 2 * DAY_SECS).unwrap();
        assert_eq!((decks[0].due, decks[0].new), (2, 0));
        let today = inner.today(T0).unwrap();
        assert_eq!((today.new_studied, today.new_available), (2, 2));
        assert!(matches!(
            inner.answer("nope", Rating::Good, T0),
            Err(AmgiError::UnknownCard { .. })
        ));

        // Undo takes answers back latest first, leaving the cards as they were.
        let undone = inner.undo_answer(T0).unwrap().unwrap();
        assert_eq!(undone, card2);
        let undone = inner.undo_answer(T0).unwrap().unwrap();
        assert_eq!(undone, card);
        assert!(inner.states.is_empty());
        assert_eq!(inner.today(T0).unwrap().new_studied, 0);
        assert!(inner.undo_answer(T0).unwrap().is_none());
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn review_order() {
        let (dir, c) = collection(&[
            ("a.md", "Q: 1\nA: -\n\nQ: 2\nA: -\n\nQ: 3\nA: -\n"),
            ("b.md", "Q: 4\nA: -\n\nQ: 5\nA: -\n\nQ: 6\nA: -\n"),
        ]);
        let mut inner = c.lock();
        let ids: Vec<String> = inner.cards.iter().map(|c| c.parsed.id.clone()).collect();
        // Card 6 is studied a day before the others, so it falls due a day earlier.
        inner.answer(&ids[5], Rating::Good, T0 - DAY_SECS).unwrap();
        for id in &ids[..5] {
            inner.answer(id, Rating::Good, T0).unwrap();
        }
        let later = T0 + 60 * DAY_SECS;
        let next = |inner: &Inner, now| inner.next_card(&StudyScope::All, now).unwrap().unwrap().id;

        // The most overdue day first, then a fixed shuffle within the day, the same
        // each time it is asked.
        assert_eq!(next(&inner, later), ids[5]);
        inner.answer(&ids[5], Rating::Good, later).unwrap();
        let key = |id: &String| parser::stable_hash(&[id, &T0.to_string()]);
        let mut expected = ids[..5].to_vec();
        expected.sort_by_key(key);
        assert_eq!(next(&inner, later), expected[0]);
        assert_eq!(next(&inner, later), expected[0]);

        // A failed card comes back after its delay, ahead of the reviews still due.
        inner.answer(&expected[0], Rating::Again, later).unwrap();
        assert_eq!(next(&inner, later), expected[1]);
        assert_eq!(next(&inner, later + scheduler::AGAIN_DELAY_SECS), expected[0]);

        // A review due at the next day's start, only hours after it was studied, is
        // still a review: it doesn't jump ahead of one due days earlier.
        let state = |due, last_review, interval_secs| CardState {
            memory: Memory { stability: 1.0, difficulty: 5.0 },
            due,
            last_review,
            reps: 1,
            lapses: 0,
            interval_secs,
        };
        let hour = 3600;
        inner.states.clear();
        inner.states.insert(ids[0].clone(), state(later + 3 * hour, later - 3 * hour, DAY_SECS));
        inner.states.insert(ids[1].clone(), state(later - 3 * DAY_SECS, later - 4 * DAY_SECS, DAY_SECS));
        assert_eq!(next(&inner, later + 4 * hour), ids[1]);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn database_deleted_or_replaced_while_open() {
        let (dir, _) = collection(&[("a.md", "Q: 1\nA: one\n")]);
        let c = CardCollection::open(dir.to_string_lossy().into_owned()).unwrap();
        let db = dir.join(DB_FILE);
        let id = c.cards(StudyScope::All)[0].id.clone();
        c.answer(id.clone(), Rating::Good).unwrap();
        let saved = dir.join("saved.sqlite");
        fs::copy(&db, &saved).unwrap();
        let reps = |c: &CardCollection| c.cards(StudyScope::All)[0].reps;
        assert_eq!(reps(&c), 1);
        assert!(c.warnings().is_empty());

        // Deleted: a new database, as after a restart, and new reviews land in it.
        fs::remove_file(&db).unwrap();
        assert_eq!(reps(&c), 0);
        assert!(c.warnings()[0].message.contains("deleted"));
        c.answer(id.clone(), Rating::Good).unwrap();
        assert_eq!(Store::open(&db).unwrap().reviews(&id).unwrap().len(), 1);

        // Replaced, as by a sync or restore: its history is used.
        c.answer(id.clone(), Rating::Good).unwrap();
        assert_eq!(reps(&c), 2);
        fs::rename(&saved, &db).unwrap();
        assert_eq!(reps(&c), 1);
        assert!(c.warnings()[0].message.contains("replaced"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn browse_cards_as_json_and_by_id() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: one\n#x\n\nQ: 2\nA: two\n")]);
        let cards = c.cards(StudyScope::All);
        let json: serde_json::Value = serde_json::from_str(&c.cards_json(StudyScope::All)).unwrap();
        assert_eq!(json[0]["front"], "1");
        assert_eq!(json[0]["kind"], "basic");
        assert_eq!(json[0]["tags"], serde_json::json!(["x"]));
        assert_eq!(json[1]["due"], serde_json::Value::Null);
        let found = c.cards_with_ids(vec![cards[1].id.clone(), "missing".into()]);
        assert_eq!(found, [cards[1].clone()]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn browse_lists_cards_with_state() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: one\n#x\n\nQ: 2\nA: two\n"), ("b.md", "Q: 3\nA: three\n")]);
        let mut inner = c.lock();
        let first = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        inner.answer(&first.id, Rating::Again, T0).unwrap();

        let cards = inner.browse(&deck("a"));
        let fronts: Vec<_> = cards.iter().map(|c| c.front.as_str()).collect();
        assert_eq!(fronts, ["1", "2"]);
        assert_eq!(cards[0].tags, ["x"]);
        assert_eq!((cards[0].reps, cards[0].due.is_some()), (1, true));
        assert_eq!((cards[1].reps, cards[1].due), (0, None));
        assert_eq!(inner.browse(&StudyScope::All).len(), 3);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn card_info_has_schedule_and_history() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: one\n#x\n")]);
        let mut inner = c.lock();
        let id = inner.next_card(&deck("a"), T0).unwrap().unwrap().id;

        let new = inner.card_info(&id, T0).unwrap().unwrap();
        assert_eq!((new.front.as_str(), new.tags.as_slice(), new.due), ("1", ["x".to_string()].as_slice(), None));
        assert_eq!((new.stability, new.retrievability, new.reviews.len()), (None, None, 0));

        inner.answer(&id, Rating::Good, T0).unwrap();
        inner.answer(&id, Rating::Again, T0 + 3 * DAY_SECS).unwrap();
        let info = inner.card_info(&id, T0 + 3 * DAY_SECS).unwrap().unwrap();
        assert_eq!((info.reps, info.lapses), (2, 1));
        assert_eq!(info.reviews.iter().map(|r| r.rating).collect::<Vec<_>>(), [Rating::Good, Rating::Again]);
        assert!(info.reviews[0].was_new && info.reviews[1].elapsed_days == 3.0);
        assert_eq!(info.due, Some(T0 + 3 * DAY_SECS + scheduler::AGAIN_DELAY_SECS));
        assert!((info.retrievability.unwrap() - 1.0).abs() < 1e-6);
        let later = inner.card_info(&id, T0 + 10 * DAY_SECS).unwrap().unwrap();
        assert!(later.retrievability.unwrap() < 1.0);

        // Review stages: a lapse seen again the same day is relearned.
        inner.answer(&id, Rating::Good, T0 + 3 * DAY_SECS + 600).unwrap();
        let states: Vec<_> = inner.store.reviews(&id).unwrap().iter().map(|r| r.state).collect();
        use ReviewState::*;
        assert_eq!(states, [Some(New), Some(Review), Some(Relearning)]);
        assert!(inner.card_info("nope", T0).unwrap().is_none());
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decks_contain_subdecks() {
        let (dir, c) = collection(&[
            ("lang.md", "Q: l\nA: 1\n"),
            ("lang/ko.md", "Q: k\nA: 2\n"),
            ("lang/ko/verbs.md", "Q: v\nA: 3\n"),
            ("language.md", "Q: x\nA: 4\n"),
        ]);
        let inner = c.lock();
        let decks = inner.decks(T0).unwrap();
        let summary: Vec<_> = decks.iter().map(|d| (d.name.as_str(), d.total)).collect();
        assert_eq!(summary, [("lang", 3), ("lang/ko", 2), ("lang/ko/verbs", 1), ("language", 1)]);
        let fronts: Vec<_> = inner.in_scope(&deck("lang/ko")).map(|c| c.parsed.front.as_str()).collect();
        assert_eq!(fronts, ["k", "v"]);
        assert_eq!(inner.in_scope(&deck("lang")).count(), 3);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_cards() {
        let (dir, c) = collection(&[("a.md", "+++\ntags = [\"lang\"]\n+++\nQ: one\nA: 1\n   \n\n"), ("b.md", "```\nopen")]);
        let new = |deck: &str, kind, front: &str| NewCard {
            deck: deck.into(),
            kind,
            front: front.into(),
            back: "2".into(),
            tags: vec!["#numbers".into()],
        };
        let ids = c.add_card(new("a", CardKind::Basic, "two")).unwrap();
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "+++\ntags = [\"lang\"]\n+++\nQ: one\nA: 1\n\nQ: two\nA: 2\n#numbers\n");
        let info = c.card_info(ids[0].clone()).unwrap().unwrap();
        assert_eq!(info.tags, ["lang", "numbers"]);

        // A new file in a new folder; a cloze makes a card per gap.
        let ids = c.add_card(new("sub/deck", CardKind::Cloze, "[Seoul] is in [Korea].")).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(fs::read_to_string(dir.join("sub/deck.md")).unwrap(), "C: [Seoul] is in [Korea].\n#numbers\n");

        let err = |card| match c.add_card(card) {
            Err(AmgiError::InvalidCard { message }) => message,
            other => panic!("{other:?}"),
        };
        assert_eq!(err(new("b", CardKind::Basic, "two")), "This card is already in a.md.");
        assert_eq!(err(new("../x", CardKind::Basic, "three")), "Choose a deck for the card.");
        // An unclosed code block would take the card in; the file is left as it was.
        assert!(err(new("b", CardKind::Basic, "three")).contains("unclosed code block"));
        assert_eq!(fs::read_to_string(dir.join("b.md")).unwrap(), "```\nopen");
        drop(c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dashboard_stats() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n")]);
        let mut inner = c.lock();
        let ids: Vec<String> = inner.cards.iter().map(|c| c.parsed.id.clone()).collect();
        // Study on three consecutive days, ending yesterday.
        inner.answer(&ids[0], Rating::Good, T0 - 2 * DAY_SECS).unwrap();
        inner.answer(&ids[1], Rating::Easy, T0 - DAY_SECS).unwrap();
        inner.answer(&ids[0], Rating::Again, T0 - DAY_SECS + 60).unwrap();

        let d = inner.dashboard(5, T0).unwrap();
        assert_eq!(d.activity, [0, 0, 1, 2, 0]);
        assert_eq!(inner.day_start(T0) - d.activity_start, 4 * DAY_SECS);
        assert_eq!(d.streak, 2);
        assert_eq!((d.new_cards, d.young_cards, d.mature_cards), (1, 2, 0));
        assert_eq!(d.forecast.len(), FORECAST_DAYS);
        // The failed card is due today; the Easy one later.
        assert_eq!(d.forecast[0], 1);
        assert_eq!(d.forecast.iter().sum::<u32>(), 2);
        // The Again was a review a day after the first one.
        assert_eq!(d.retention, Some(0.0));

        inner.answer(&ids[2], Rating::Good, T0).unwrap();
        let d = inner.dashboard(5, T0).unwrap();
        assert_eq!((d.activity[4], d.streak), (1, 3));
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tags_and_titles() {
        let (dir, c) = collection(&[
            ("ko.md", "+++\nname = \"Korean\"\ntags = [\"lang\"]\n+++\nQ: 가다\nA: go\n#verbs\n\nQ: 집\nA: house\n"),
            ("en.md", "Q: run\nA: бегать\n#Verbs #lang\n"),
        ]);
        let inner = c.lock();
        let tags: Vec<_> = inner.tags().into_iter().map(|t| (t.name, t.cards)).collect();
        assert_eq!(tags, [("lang".into(), 3), ("Verbs".into(), 2)]);
        let titles: Vec<_> = inner.decks(T0).unwrap().into_iter().map(|d| d.title).collect();
        assert_eq!(titles, ["en", "Korean"]);
        let card = inner.next_card(&StudyScope::All, T0).unwrap().unwrap();
        assert_eq!(card.tags, ["Verbs", "lang"]);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_card_limit() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n")]);
        c.set_new_cards_per_day(1);
        let mut inner = c.lock();
        let card = inner.next_card(&StudyScope::All, T0).unwrap().unwrap();
        inner.answer(&card.id, Rating::Easy, T0).unwrap();
        assert!(inner.next_card(&StudyScope::All, T0).unwrap().is_none());
        assert_eq!(inner.decks(T0).unwrap()[0].new, 0);
        // Tomorrow the limit resets.
        assert!(inner.next_card(&StudyScope::All, T0 + DAY_SECS).unwrap().is_some());
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn suspended_cards_are_not_studied() {
        let (dir, c) = collection(&[
            ("a.md", "Q: kept\nA: 1\n\nQ: paused\nA: 2\n#suspended\n"),
            ("b.md", "+++\ntags = [\"suspended\"]\n+++\nQ: whole deck\nA: 3\n"),
        ]);
        let ids: Vec<String> = c.cards(StudyScope::All).into_iter().map(|card| card.id).collect();
        let mut inner = c.lock();
        assert_eq!(inner.decks(T0).unwrap().iter().map(|d| d.new).sum::<u32>(), 1);
        assert_eq!(inner.today(T0).unwrap().new_available, 1);
        let card = inner.next_card(&StudyScope::All, T0).unwrap().unwrap();
        assert_eq!(card.front, "kept");
        inner.answer(&card.id, Rating::Good, T0).unwrap();
        assert!(inner.next_card(&StudyScope::All, T0).unwrap().is_none());
        let browsed = inner.browse(&StudyScope::All);
        assert_eq!(browsed.iter().map(|c| c.suspended).collect::<Vec<_>>(), [false, true, true]);
        assert_eq!(inner.dashboard(1, T0).unwrap().suspended_cards, 2);

        // Unsuspending edits the file; a front matter tag stays.
        assert_eq!(inner.set_suspended(&ids[1..], false).unwrap(), 1);
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "Q: kept\nA: 1\n\nQ: paused\nA: 2\n");
        assert_eq!(inner.next_card(&StudyScope::All, T0).unwrap().unwrap().front, "paused");
        // And suspending puts the tag back, keeping the card's history.
        assert_eq!(inner.set_suspended(&ids[..1], true).unwrap(), 1);
        assert!(inner.states.contains_key(&ids[0]));
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "Q: kept\nA: 1\n#suspended\n\nQ: paused\nA: 2\n");
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn day_rollover_respects_offset() {
        let (dir, c) = collection(&[("a.md", "")]);
        c.set_utc_offset(5 * 3600);
        let inner = c.lock();
        // 2025-10-09 03:00 local (+05:00) is still the previous study day.
        let start = inner.day_start(1_759_960_800);
        assert_eq!((start + 5 * 3600 - DAY_ROLLOVER_SECS).rem_euclid(DAY_SECS), 0);
        assert!(start <= 1_759_960_800 && 1_759_960_800 - start < DAY_SECS);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_writes_decks_and_keeps_history() {
        let (dir, c) = collection(&[("Default.md", "Q: mine\nA: kept\n")]);
        let studied = "Q: studied\nA: yes\n";
        let id = parser::parse(studied).cards[0].id.clone();
        let review = |day, interval_days| HistoryEntry::Review {
            time: T0 + day * DAY_SECS,
            rating: Rating::Good,
            state: Some(ReviewState::Learning),
            interval_secs: Some(interval_days * DAY_SECS),
        };
        let import = || Import {
            decks: vec![("Default".into(), studied.into()), ("Lang/Verbs".into(), "Q: fresh\nA: no\n".into())],
            note_types: Vec::new(),
            histories: vec![(id.clone(), vec![review(0, 1), review(1, 3)]), (id.clone(), vec![review(0, 1)])],
        };
        let imported = c.import(import()).unwrap();
        assert_eq!(imported.files, ["Default 2.md", "Lang/Verbs.md"]);
        assert_eq!((imported.reviewed_cards, imported.existing_history, imported.new_cards), (1, 0, 1));
        assert_eq!(fs::read_to_string(dir.join("Default.md")).unwrap(), "Q: mine\nA: kept\n");

        let inner = c.lock();
        let names: Vec<_> = inner.decks(T0).unwrap().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Default", "Default 2", "Lang", "Lang/Verbs"]);
        let state = inner.states[&id];
        assert_eq!((state.reps, state.last_review, state.due), (2, T0 + DAY_SECS, T0 + 4 * DAY_SECS));
        assert_eq!(inner.store.state(&id).unwrap(), Some(state));
        drop(inner);

        // Importing again keeps the history already here and says so.
        let again = c.import(import()).unwrap();
        assert_eq!((again.reviewed_cards, again.existing_history, again.new_cards), (0, 1, 1));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn notes_use_note_types_from_their_folder() {
        let vocab = "fields = [\"Word\", \"Meaning\"]\ncards = [\"Word -> Meaning\", \"Meaning -> Word\"]\n";
        let (dir, c) = collection(&[
            ("note-types/Vocab.toml", vocab),
            ("note-types/Bad.toml", "fields = [\"Only\"]\n"),
            ("note-types/README.md", "Q: not a deck\nA: -\n"),
            ("es.md", "@Vocab\nWord: uno\nMeaning: one\n@Vocab\nWord: dos\nMeaning: two\n\nQ: plain\nA: card\n"),
            ("bad.md", "Q: a\nA: b\n\n@Bad\nOnly: x\n"),
        ]);
        let warnings: Vec<_> = c.warnings().into_iter().map(|w| format!("{}:{}: {}", w.path, w.line, w.message)).collect();
        assert_eq!(
            warnings,
            [
                "note-types/Bad.toml:1: notes need two fields, or `cards` to say which make a card",
                "bad.md:4: note type `Bad` has errors, see `note-types/Bad.toml`",
            ]
        );
        // Browsed cards carry their note's fields, and the note types say what they are.
        let browsed = c.cards(StudyScope::All);
        let dos = browsed.iter().find(|b| b.front == "dos").unwrap();
        assert_eq!((dos.note_type.as_deref(), dos.fields.clone()), (Some("Vocab"), vec!["dos".to_string(), "two".into()]));
        assert!(c.cards_json(StudyScope::All).contains(r#""noteType":"Vocab","fields":["dos","two"]"#));
        assert_eq!(c.note_types_json(), r#"[{"name":"Vocab","fields":["Word","Meaning"]}]"#);

        let inner = c.lock();
        let fronts: Vec<_> = inner.cards.iter().filter(|c| c.path == "es.md").map(|c| c.parsed.front.as_str()).collect();
        assert_eq!(fronts, ["uno", "one", "dos", "two", "plain"]);
        let two = inner.cards[3].parsed.id.clone();
        drop(inner);
        // The deck's note type is the one most of its notes have.
        let decks = c.decks().unwrap();
        assert_eq!(decks.iter().find(|d| d.name == "es").unwrap().note_type.as_deref(), Some("Vocab"));
        assert_eq!(decks.iter().find(|d| d.name == "bad").unwrap().note_type, None);

        // Suspending a card suspends its note.
        assert_eq!(c.set_suspended(vec![two], true).unwrap(), 1);
        assert!(fs::read_to_string(dir.join("es.md")).unwrap().contains("Word: dos\nMeaning: two\n#suspended\n\nQ: plain"));

        // An imported note type like one already here shares its file; a different one
        // with the same name gets a file of its own, and its notes name that.
        let deck = |name: &str, word: &str| format!("@{name}\nWord: {word}\nMeaning: -\n");
        let import = Import {
            decks: vec![("a".into(), deck("Vocab", "tres")), ("b".into(), deck("Other", "cuatro"))],
            note_types: vec![("Vocab".into(), vocab.into()), ("Other".into(), vocab.into())],
            histories: Vec::new(),
        };
        let other = Import {
            decks: vec![("c".into(), deck("Vocab", "cinco"))],
            note_types: vec![("Vocab".into(), "fields = [\"Word\", \"Meaning\"]\n".into())],
            histories: Vec::new(),
        };
        assert_eq!(c.import(import).unwrap().files, ["note-types/Other.toml", "a.md", "b.md"]);
        assert_eq!(c.import(other).unwrap().files, ["note-types/Vocab 2.toml", "c.md"]);
        assert_eq!(fs::read_to_string(dir.join("c.md")).unwrap(), "@Vocab 2\nWord: cinco\nMeaning: -\n");
        let inner = c.lock();
        let fronts: Vec<_> = inner.cards.iter().filter(|c| c.path == "c.md").map(|c| c.parsed.front.as_str()).collect();
        assert_eq!(fronts, ["cinco"]);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_notes_and_media() {
        let (dir, c) = collection(&[
            ("note-types/Vocab.toml", "fields = [\"Word\", \"Meaning\", \"Audio\"]\ncards = [\"Word, Audio -> Meaning\", \"Meaning -> Word\"]\n"),
            ("lang/es.md", "+++\nname = \"Spanish\"\n+++\n"),
        ]);

        let sound = dir.join("hola (1).mp3");
        fs::write(&sound, b"sound").unwrap();
        let link = c.add_media("lang/es".into(), sound.to_string_lossy().into_owned()).unwrap();
        assert_eq!(link, "![](<../media/hola (1).mp3>)");
        // The same file again is shared; another with its name is kept beside it.
        assert_eq!(c.add_media("lang/es".into(), sound.to_string_lossy().into_owned()).unwrap(), link);
        fs::write(&sound, b"other").unwrap();
        assert_eq!(c.add_media("es".into(), sound.to_string_lossy().into_owned()).unwrap(), "![](<media/hola (1) 2.mp3>)");

        let values = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(c.note_card_count("Vocab".into(), values(&["hola", "hello", ""])), 2);
        assert_eq!(c.note_card_count("Vocab".into(), values(&["hola", "", ""])), 0);
        assert_eq!(c.note_card_count("Nope".into(), values(&["hola", "hello", ""])), 0);
        let note = NewNote {
            deck: "lang/es".into(),
            note_type: "Vocab".into(),
            values: values(&["hola", "hello", &link]),
            tags: vec!["greeting".into()],
        };
        assert_eq!(c.add_note(note.clone()).unwrap().len(), 2);
        assert_eq!(
            fs::read_to_string(dir.join("lang/es.md")).unwrap(),
            "+++\nname = \"Spanish\"\n+++\n\n@Vocab\nWord: hola\nMeaning: hello\nAudio: ![](<../media/hola (1).mp3>)\n#greeting\n"
        );
        assert!(matches!(c.add_note(note), Err(AmgiError::InvalidCard { message }) if message.contains("lang/es.md:5")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_anki_package() {
        let (dir, c) = collection(&[("Default.md", "Q: mine\nA: kept\n")]);
        let package = dir.join("deck.apkg");
        anki::tests::write_package(&package);
        let summary = c.import_anki(package.to_string_lossy().into_owned()).unwrap();
        assert_eq!(summary.files, ["Default 2.md", "Lang/Kazakh-Verbs.md"]);
        assert_eq!((summary.cards, summary.reviewed_cards, summary.skipped_notes), (5, 3, 1));
        assert_eq!((summary.existing_history, summary.new_cards, summary.suspended_cards), (0, 2, 2));
        assert_eq!((summary.images, summary.missing_images), (1, 0));
        assert_eq!((summary.sounds, summary.missing_sounds), (1, 1));
        assert_eq!(fs::read(dir.join("media/hi.png")).unwrap(), b"png bytes");
        assert_eq!(fs::read(dir.join("media/paris.mp3")).unwrap(), b"mp3 bytes");

        let inner = c.lock();
        assert_eq!(fs::read_to_string(dir.join("Default.md")).unwrap(), "Q: mine\nA: kept\n");
        let names: Vec<_> = inner.decks(T0).unwrap().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Default", "Default 2", "Lang", "Lang/Kazakh-Verbs"]);
        assert_eq!(inner.decks(T0).unwrap()[3].title, "Kazakh/Verbs");
        let names: Vec<_> = inner.tags().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["geo", "suspended"]);
        let card = inner.cards.iter().find(|c| c.parsed.front == "сәлем").unwrap();
        let state = inner.states[&card.parsed.id];
        // Due on the card's day in Anki (day 5), not 3 days after its last review as logged.
        assert_eq!((state.reps, state.last_review, state.due), (2, 1_700_086_400, 1_699_920_000 + 5 * DAY_SECS));
        assert_eq!(inner.store.state(&card.parsed.id).unwrap(), Some(state));
        drop(inner);

        // Importing again keeps the history already here and says so.
        let again = c.import_anki(package.to_string_lossy().into_owned()).unwrap();
        assert_eq!((again.reviewed_cards, again.existing_history, again.new_cards), (0, 3, 2));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replay_follows_imported_schedule() {
        let (dir, c) = collection(&[("a.md", "")]);
        let inner = c.lock();
        let review = |day: i64, rating, state, interval_secs| HistoryEntry::Review {
            time: T0 + day * DAY_SECS,
            rating,
            state,
            interval_secs: Some(interval_secs),
        };
        let reviews = |entries: Vec<LogEntry>| -> Vec<LoggedReview> {
            entries.into_iter().filter_map(|e| if let LogEntry::Review(r) = e { Some(r) } else { None }).collect()
        };
        use Rating::{Again, Good};
        use ReviewState::*;
        let log = [
            review(0, Again, Some(Learning), 600),
            review(0, Good, Some(Learning), DAY_SECS),
            review(1, Good, Some(Review), 4 * DAY_SECS),
            review(5, Again, Some(Review), 600),
            review(5, Good, Some(Relearning), 2 * DAY_SECS),
        ];
        let (entries, state) = inner.replay(&log).unwrap().unwrap();
        let logged = reviews(entries);
        assert_eq!(logged.len(), 5);
        assert!(logged[0].was_new && !logged[1].was_new);
        assert_eq!(logged.iter().map(|r| r.elapsed_days).collect::<Vec<_>>(), [0.0, 0.0, 1.0, 4.0, 0.0]);
        assert_eq!(logged.iter().map(|r| r.state).collect::<Vec<_>>(), [
            Some(New),
            Some(Learning),
            Some(Review),
            Some(Review),
            Some(Relearning)
        ]);
        assert_eq!((state.reps, state.lapses, state.due), (5, 1, T0 + 7 * DAY_SECS));
        assert!(state.memory.stability > 0.0);

        // A history that starts mid-way has no New review.
        let (entries, _) = inner.replay(&log[2..]).unwrap().unwrap();
        let logged = reviews(entries);
        assert!(logged[0].was_new);
        assert_eq!(logged[0].state, Some(Review));

        // With no stage given, it follows from the reviews before; with no interval, the
        // scheduler picks one.
        let unknown = review(3, Good, None, 5 * DAY_SECS);
        let (entries, state) = inner.replay(&[log[1], unknown]).unwrap().unwrap();
        assert_eq!(reviews(entries).iter().map(|r| r.state).collect::<Vec<_>>(), [Some(New), Some(Review)]);
        assert_eq!((state.reps, state.due), (2, T0 + 8 * DAY_SECS));
        let unscheduled = HistoryEntry::Review { time: T0, rating: Good, state: None, interval_secs: None };
        let (entries, state) = inner.replay(&[unscheduled]).unwrap().unwrap();
        assert_eq!(state.due, T0 + reviews(entries)[0].interval_secs);

        // Rescheduling moves the card; a reset makes it new again and is logged.
        let moved = HistoryEntry::Reschedule { time: T0 + DAY_SECS, interval_secs: 10 * DAY_SECS };
        let (_, state) = inner.replay(&[log[1], moved]).unwrap().unwrap();
        assert_eq!(state.due, T0 + 11 * DAY_SECS);
        let reset = HistoryEntry::Reset { time: T0 + DAY_SECS };
        assert!(inner.replay(&[log[1], reset]).unwrap().is_none());
        let (entries, state) =
            inner.replay(&[log[1], reset, review(2, Good, Some(Learning), DAY_SECS)]).unwrap().unwrap();
        assert_eq!(entries[1], LogEntry::Reset(T0 + DAY_SECS));
        let logged = reviews(entries);
        assert!(logged[1].was_new);
        assert_eq!(logged[1].state, Some(New));
        assert_eq!(state.reps, 1);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn intervals_format() {
        assert_eq!(format_interval(600), "10m");
        assert_eq!(format_interval(5400), "1.5h");
        assert_eq!(format_interval(86_400 * 3), "3d");
        assert_eq!(format_interval(86_400 * 45), "1.5mo");
        assert_eq!(format_interval(86_400 * 800), "2.2y");
    }
}
