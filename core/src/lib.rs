//! amgi: plain-text flash cards scheduled with FSRS.
//!
//! A collection is a directory of Markdown files; each file is a deck. Review
//! state lives in `.amgi.sqlite` at the collection root.

pub mod anki;
pub mod anki_connect;
pub mod config;
pub mod format;
pub mod lint;
pub mod media;
pub mod parser;
pub mod scheduler;
pub mod scripts;
pub mod search;
pub mod settings;
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
use scheduler::{Memory, PlannedCard, RatingHabits, Scheduler};
use search::Query;
use store::{CardState, CardTime, LogEntry, LoggedReview, Store};

uniffi::setup_scaffolding!();

pub const DB_FILE: &str = ".amgi.sqlite";
/// Folder at the collection root that imported images are copied to.
pub const MEDIA_DIR: &str = "media";
/// How the app links to a file in `media`: from the collection root, so the link works
/// from any deck and keeps working when the deck moves.
pub(crate) const MEDIA_LINK: &str = "@/media/";
/// Folder at the collection root that holds the decks; `media` and `note-types` sit beside it.
pub const CARDS_DIR: &str = "cards";
const DAY_SECS: i64 = 86_400;
/// The study day rolls over at 4am local time unless set otherwise, so late-night
/// study counts as the same day.
const DEFAULT_ROLLOVER_HOUR: u8 = 4;
/// When nothing else is left, a failed card due back within this window is shown early.
const LEARN_AHEAD_SECS: i64 = 20 * 60;
/// Keys of the settings the collection's database kept before `amgi.toml` did, moved
/// to the file when the collection opens.
const RETENTION_KEY: &str = "desired_retention";
const ROLLOVER_HOUR_KEY: &str = "rollover_hour";
const REVIEWS_PER_DAY_KEY: &str = "reviews_per_day";
const NEW_PER_DAY_KEY: &str = "new_cards_per_day";
/// New cards a day allows when the collection hasn't set its own limit, as in Anki.
const DEFAULT_NEW_PER_DAY: u32 = 20;
const LEECH_THRESHOLD_KEY: &str = "leech_threshold";
const LEECH_SUSPEND_KEY: &str = "leech_suspend";
/// Lapses that make a card a leech when the collection hasn't set its own, as in Anki.
pub const DEFAULT_LEECH_THRESHOLD: u32 = 8;
/// The most time one review counts, as Anki does by default: past this, the person
/// has most likely stepped away with the card on screen.
pub const MAX_ANSWER_MS: u32 = 60_000;
/// Days of timed reviews that average time per card is taken over.
const TIME_AVERAGE_DAYS: i64 = 30;

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
    /// The AnkiConnect server couldn't start, with a sentence saying why.
    #[error("{message}")]
    AnkiConnect { message: String },
    /// A card that can't be added, with a sentence saying why.
    #[error("{message}")]
    InvalidCard { message: String },
    /// `~/.config/amgi/config.toml` can't be read, with a sentence naming the file and why.
    #[error("{message}")]
    Config { message: String },
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
    /// Cards answered Again and not yet passed, whether waiting minutes to come back or
    /// back already. Not capped by the daily review limit.
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
    pub stage: StudyStage,
    /// Seconds until the card is due again for Again, Hard, Good, Easy.
    pub intervals: Vec<i64>,
    /// The answer to type before it shows, as Markdown; `None` unless the card asks for one.
    pub typed: Option<String>,
    /// The field that answer comes from, such as `Word`, which the typing box names.
    pub typed_field: Option<String>,
    /// The hint of a typed gap, `[=answer|hint]`, which the typing box shows.
    pub typed_hint: Option<String>,
}

/// Which of today's counts a card to study comes from, as the deck list shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StudyStage {
    /// Never studied.
    New,
    /// Answered Again, back after minutes.
    Learning,
    /// Back after an interval of days.
    Due,
}

/// A card with its schedule, as `amgi search` and AnkiConnect list it.
#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct CardSummary {
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
    /// Seconds from the last review to the due date; `None` for a new card.
    pub interval: Option<i64>,
    pub reps: u32,
    pub lapses: u32,
    /// Tagged `#suspended`, so never studied.
    pub suspended: bool,
    /// Forgotten as many times as the leech threshold, or more.
    pub leech: bool,
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
    /// Milliseconds spent on the card before rating it; `None` when not known.
    pub taken_ms: Option<u32>,
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
    /// Reviews due now, capped by the daily review limit.
    pub due_available: u32,
    /// Cards answered Again and not yet passed, as in `DeckInfo::learning`.
    pub learning: u32,
    /// Time spent on today's reviews, counting only those timed.
    pub study_secs: u32,
    /// About how long the cards left today will take, from the time recent days took
    /// per card; `None` without timed reviews in the last 30 days.
    pub secs_left: Option<u32>,
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
        /// Milliseconds spent on the card before rating it, if the other app knows.
        taken_ms: Option<u32>,
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

/// Statistics for a deck or the whole collection, as `amgi stats` shows them.
#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct Stats {
    /// Reviews per study day, oldest first, ending today.
    pub activity: Vec<u32>,
    /// Seconds spent on each day of `activity`, counting only timed reviews.
    pub activity_secs: Vec<u32>,
    /// Average seconds per review over the last 30 days; `None` without timed reviews.
    pub secs_per_review: Option<f64>,
    /// Average seconds a card studied before takes on the day it comes back, all its
    /// reviews that day together, over the last 30 days. Estimates time for `forecast`.
    pub secs_per_due_card: Option<f64>,
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
    /// Reviews of cards scheduled a day or more ahead, and how many were remembered.
    pub retention: RetentionTable,
}

/// Recall over recent study days, each period ending today.
#[derive(Debug, Clone, Copy, Default, PartialEq, uniffi::Record, serde::Serialize)]
pub struct RetentionTable {
    pub today: Retention,
    pub yesterday: Retention,
    /// The last 7 days.
    pub week: Retention,
    /// The last 30 days.
    pub month: Retention,
    /// The last 365 days.
    pub year: Retention,
}

/// Reviews in a period of cards the review before scheduled a day or more ahead, as in
/// Anki: not a failed card back after minutes. Passed is any rating but Again. A card
/// is mature when it was scheduled 21 days or more ahead.
#[derive(Debug, Clone, Copy, Default, PartialEq, uniffi::Record, serde::Serialize)]
pub struct Retention {
    pub young_passed: u32,
    pub young_failed: u32,
    pub mature_passed: u32,
    pub mature_failed: u32,
}

impl Retention {
    fn add(&mut self, mature: bool, passed: u32, failed: u32) {
        let (p, f) = if mature {
            (&mut self.mature_passed, &mut self.mature_failed)
        } else {
            (&mut self.young_passed, &mut self.young_failed)
        };
        *p += passed;
        *f += failed;
    }
}

const MATURE_SECS: i64 = 21 * DAY_SECS;
const FORECAST_DAYS: usize = 14;
/// Ratings needed before the simulator goes by the collection's own rather than the
/// defaults.
const MIN_RATINGS: u32 = 100;

/// Reviews expected in the coming days, simulated, as `amgi forecast` shows them.
#[derive(Debug, Clone, PartialEq, uniffi::Record, serde::Serialize)]
pub struct Workload {
    /// Reviews expected each day from today of the cards studied before, averaged over
    /// simulated runs; today includes overdue cards. Not held to the daily review limit.
    pub reviews: Vec<f64>,
    /// Reviews the new cards add each day after the day each is first studied.
    pub added_reviews: Vec<f64>,
    /// New cards first studied today, or each day when `every_day`.
    pub new_cards: u32,
    pub every_day: bool,
    /// With `every_day`, as if new cards are added as fast, so they never run out.
    pub keep_adding: bool,
    /// New cards first studied on each day, fewer once the unstudied cards run out.
    pub new_studied: Vec<u32>,
    /// Average seconds a card studied before takes on a day it comes back.
    pub secs_per_due_card: Option<f64>,
    /// Average seconds a new card takes the day it is first studied.
    pub secs_per_new_card: Option<f64>,
}

/// Which cards to study or list.
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
    /// New cards a day allows.
    new_per_day: u32,
    /// Lower limits for some cards, from `amgi.toml`.
    new_rules: Vec<NewRule>,
    /// Reviews of due cards a day allows; `None` for no limit.
    reviews_per_day: Option<u32>,
    /// Lapses that make a card a leech; at least 1.
    leech_threshold: u32,
    /// Whether a card is suspended when it becomes a leech.
    leech_suspend: bool,
    utc_offset_secs: i64,
    /// When the study day starts, in seconds after local midnight.
    rollover_secs: i64,
    /// Why the database was reopened while the collection was open, shown with the
    /// warnings until the app restarts.
    store_notice: Option<CollectionWarning>,
    /// Answers given since the collection was opened, latest last, for `undo_answer`.
    answered: Vec<Answered>,
    /// Every file the last load read, with its text, so a reload can skip parsing when
    /// none changed. `None` until the first load.
    sources: Option<Sources>,
}

/// At most `per_day` new cards a day from those the search `cards` matches.
struct NewRule {
    cards: String,
    per_day: u32,
    /// In `amgi.toml`.
    line: u32,
}

/// What the new-card rules leave of today's new cards.
#[derive(Default)]
struct RuleBudget {
    /// New cards each rule still allows today.
    left: Vec<u32>,
    /// The rules matching each unstudied card, by id; cards no rule matches are left out.
    rules: HashMap<String, Vec<usize>>,
}

impl RuleBudget {
    /// Whether every rule matching the card allows another new card today.
    fn allows(&self, id: &str) -> bool {
        self.rules.get(id).is_none_or(|rules| rules.iter().all(|&r| self.left[r] > 0))
    }

    /// How many of `cards`, unstudied and in order, the rules let through today, up to
    /// `cap`. Each card taken counts toward every rule matching it.
    fn available<'a>(&self, cards: impl Iterator<Item = &'a Card>, cap: u32) -> u32 {
        let mut left = self.left.clone();
        let mut taken = 0;
        for card in cards {
            if taken >= cap {
                break;
            }
            let rules = self.rules.get(&card.parsed.id).map_or(&[][..], Vec::as_slice);
            if rules.iter().all(|&r| left[r] > 0) {
                rules.iter().for_each(|&r| left[r] -= 1);
                taken += 1;
            }
        }
        taken
    }
}

/// The files a load reads, each with its path and text, in the order they're read.
#[derive(PartialEq)]
struct Sources {
    /// `amgi.toml`, if there is one.
    settings: Option<String>,
    note_types: Vec<(PathBuf, String)>,
    decks: Vec<(PathBuf, String)>,
    /// Markdown outside `cards`, checked for decks left behind there. Unreadable files
    /// are left out, as they can't hold cards.
    strays: Vec<(PathBuf, String)>,
}

/// An answer that can be taken back: the card, its review's log row and its state before.
struct Answered {
    card_id: String,
    review_id: i64,
    prev: Option<CardState>,
    /// Whether the answer made the card a leech and suspended it.
    suspended: bool,
}

/// An answer that made its card a leech, or flagged it again as one.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Leech {
    /// Times the card has been forgotten.
    pub lapses: u32,
    /// Whether it was suspended for it.
    pub suspended: bool,
}

/// Whether a card forgotten for the `lapses`th time should be flagged as a leech: on
/// reaching the threshold, then every half threshold after, as in Anki.
fn leech_flagged(lapses: u32, threshold: u32) -> bool {
    let every = (threshold / 2).max(1);
    lapses >= threshold && (lapses - threshold).is_multiple_of(every)
}

/// Lapses to count for a card in the forecast, so the simulator, which stops a card on
/// reaching `threshold`, stops it at the lapse that would suspend it next. A leech
/// that was unsuspended keeps coming back until it's flagged again.
fn lapses_toward_leech(lapses: u32, threshold: u32) -> u32 {
    let next = (lapses + 1..).find(|&n| leech_flagged(n, threshold)).unwrap_or(threshold);
    threshold - (next - lapses)
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

    /// Re-read all Markdown files. Call after files change on disk. Returns whether
    /// any changed; if none did, the cards are left as they were.
    pub fn reload(&self) -> Result<bool> {
        self.lock().reload()
    }

    /// Lapses that make a card a leech. Collection-wide.
    pub fn leech_threshold(&self) -> u32 {
        self.lock().leech_threshold
    }

    /// Whether a card is suspended when it becomes a leech.
    pub fn suspends_leeches(&self) -> bool {
        self.lock().leech_suspend
    }

    /// Local time zone offset, used to decide when the study day rolls over.
    pub fn set_utc_offset(&self, seconds: i32) {
        self.lock().utc_offset_secs = seconds.into();
    }

    /// The hour (0–23, local time) a new study day starts. Cards on intervals of a day
    /// or more are due from the start of their day.
    pub fn rollover_hour(&self) -> u8 {
        (self.lock().rollover_secs / 3600) as u8
    }

    /// How many cards that came due a study day allows reviewing; `None` for no limit.
    /// Failed cards coming back and new cards don't count.
    pub fn reviews_per_day(&self) -> Option<u32> {
        self.lock().reviews_per_day
    }

    /// When the current study day started (Unix seconds), for searches like `is:due`.
    pub fn day_start(&self) -> i64 {
        let inner = self.lock();
        inner.day_start(unix_now())
    }

    /// Target probability of recall when a card comes due, between 0.7 and 0.99.
    pub fn desired_retention(&self) -> f32 {
        self.lock().scheduler.desired_retention
    }

    /// Every deck and folder, parents before children. Counts include subdecks.
    pub fn decks(&self) -> Result<Vec<DeckInfo>> {
        self.lock().decks(unix_now())
    }

    pub fn next_card(&self, scope: StudyScope) -> Result<Option<StudyCard>> {
        self.lock().next_card(&scope, unix_now())
    }

    /// Every card in `scope`, in file order.
    pub fn cards(&self, scope: StudyScope) -> Vec<CardSummary> {
        self.lock().cards(&scope)
    }

    /// Every tag in the collection, sorted, in the spelling first seen.
    pub fn tags(&self) -> Vec<TagInfo> {
        self.lock().tags()
    }

    /// Rate a card. `taken_ms` is the time spent on it, which the review log keeps up
    /// to `MAX_ANSWER_MS`; `None` when not timed.
    /// Returns `Some` when forgetting it made it a leech, at the threshold and every half
    /// threshold after; it's suspended then if `suspends_leeches`.
    pub fn answer(&self, card_id: String, rating: Rating, taken_ms: Option<u32>) -> Result<Option<Leech>> {
        self.lock().answer(&card_id, rating, taken_ms, unix_now())
    }

    /// Take back the latest answer not yet undone, removing it from the history, and
    /// unsuspend the card if the answer suspended it as a leech.
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
        parser::write_note(&note_type, &self.lock().note_types, &values, &[], "").map_or(0, |(_, ids)| ids.len() as u32)
    }

    /// The note types without errors, by name.
    pub fn note_types(&self) -> Vec<parser::NoteTypeInfo> {
        self.lock().note_types.list()
    }

    /// Copy an image or sound into the `media` folder, and return the Markdown that
    /// shows it there. A file with the same name and contents is shared; one that
    /// differs keeps its name, and the copy gets another. `deck` is unused: links
    /// start at the collection root.
    pub fn add_media(&self, deck: String, path: String) -> Result<String> {
        let _ = deck;
        let source = PathBuf::from(&path);
        let bytes = fs::read(&source)?;
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "media".into());
        let file = self.save_media(&name, &bytes)?;
        let link = format!("{MEDIA_LINK}{file}");
        Ok(if link.contains([' ', '(', ')', '<', '>']) { format!("![](<{link}>)") } else { format!("![]({link})") })
    }

    /// Today's reviews and new cards, in a deck or the whole collection.
    pub fn today(&self, scope: StudyScope) -> Result<TodayStats> {
        self.lock().today(&scope, unix_now())
    }

    /// Statistics for a deck or the whole collection, with `activity_days` days of
    /// review history.
    pub fn stats(&self, scope: StudyScope, activity_days: u32) -> Result<Stats> {
        self.lock().stats(&scope, activity_days, unix_now())
    }

    /// Reviews expected over the next `days` days, in a deck or the whole collection,
    /// if `new_cards` new cards are first studied today.
    /// What studying `new_cards` today, or each day from today when `every_day`,
    /// adds to the coming days' reviews. Every day's cards run out with the cards not
    /// yet studied, unless `keep_adding`.
    pub fn workload(
        &self,
        scope: StudyScope,
        new_cards: u32,
        every_day: bool,
        keep_adding: bool,
        days: u32,
    ) -> Result<Workload> {
        self.lock().workload(&scope, new_cards, every_day, keep_adding, days, unix_now())
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

    /// The deck files that aren't in `amgi fmt`'s layout. With `write`, they're
    /// rewritten, except those whose cards would change, and the collection is read again.
    pub fn format(&self, write: bool) -> Result<format::Formatted> {
        let mut inner = self.lock();
        let formatted = format::run(&inner.root, &inner.note_types, write)?;
        if write && formatted.changed.len() > formatted.refused.len() {
            inner.reload()?;
        }
        Ok(formatted)
    }

    /// What in the decks breaks `rules`, by file, line and column. With `fix`, the
    /// fixable problems are fixed in the files, unless that would change a card's id,
    /// and the collection is read again.
    pub fn lint(&self, rules: &lint::Rules, fix: bool) -> Result<Vec<lint::Problem>> {
        let mut inner = self.lock();
        let problems = lint::run(&inner.root, &inner.note_types, rules, fix)?;
        if fix && problems.iter().any(|p| p.fixable) {
            inner.reload()?;
        }
        Ok(problems)
    }

    /// How many reviews the database holds for each of `ids`, whether or not a card has
    /// that id now. Ids without reviews are left out.
    pub fn review_counts(&self, ids: &[String]) -> Result<HashMap<String, u32>> {
        Ok(self.lock().store.review_counts(ids)?)
    }

    /// Write `bytes` to the `media` folder as `name`, and return the name it was saved
    /// under. A file with that name and contents is shared; one that differs keeps its
    /// name, and the new file gets another.
    pub(crate) fn save_media(&self, name: &str, bytes: &[u8]) -> Result<String> {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
            _ => (name.to_string(), String::new()),
        };
        let dir = self.root().join(MEDIA_DIR);
        fs::create_dir_all(&dir)?;
        let mut file = name.to_string();
        for n in 2.. {
            match fs::read(dir.join(&file)) {
                Ok(existing) if existing == bytes => break,
                Ok(_) => file = format!("{stem} {n}{ext}"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::write(dir.join(&file), bytes)?;
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(file)
    }

    /// Why `card` can't be added, checked as `add_card` would without adding it.
    pub(crate) fn check_card(&self, card: &NewCard) -> Result<()> {
        self.lock().card_block(card).map(|_| ())
    }

    /// Why `note` can't be added, checked as `add_note` would without adding it.
    pub(crate) fn check_note(&self, note: &NewNote) -> Result<()> {
        self.lock().note_block(note).map(|_| ())
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
            new_per_day: DEFAULT_NEW_PER_DAY,
            new_rules: Vec::new(),
            reviews_per_day: None,
            leech_threshold: DEFAULT_LEECH_THRESHOLD,
            leech_suspend: false,
            utc_offset_secs: 0,
            rollover_secs: i64::from(DEFAULT_ROLLOVER_HOUR) * 3600,
            store_notice: None,
            answered: Vec::new(),
            sources: None,
        };
        inner.reload()?;
        inner.move_settings_to_file()?;
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

    /// Reads every file, and parses them only if one differs from the last load: a
    /// reload when nothing changed costs only the reading. Returns whether any changed.
    fn reload(&mut self) -> Result<bool> {
        let sources = self.read_sources()?;
        if self.sources.as_ref() == Some(&sources) {
            self.carry_over_history()?;
            return Ok(false);
        }

        self.cards.clear();
        self.index.clear();
        self.deck_titles.clear();
        self.deck_note_types.clear();
        self.warnings.clear();
        self.scripts = Scripts::default();
        self.load_note_types(&sources.note_types);
        self.load_settings_file(sources.settings.as_deref());
        self.warn_about_stray_decks(&sources.strays);
        let cards_dir = self.root.join(CARDS_DIR);
        for (file, text) in &sources.decks {
            let rel = relative(&self.root, file);
            let in_cards = relative(&cards_dir, file);
            let deck = in_cards.strip_suffix(".md").unwrap_or(&in_cards).to_string();
            self.scripts.scan(text);
            let result = parser::parse_deck(text, &self.note_types, &rel);
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
        self.sources = Some(sources);
        self.carry_over_history()?;
        Ok(true)
    }

    /// Every file a load reads: note types, deck files, and Markdown outside `cards`.
    /// A file that can't be read fails the load and leaves the cards as they were.
    fn read_sources(&self) -> Result<Sources> {
        let read = |files: Vec<PathBuf>| -> Result<Vec<(PathBuf, String)>> {
            files
                .into_iter()
                .map(|file| {
                    let text = fs::read_to_string(&file)?;
                    Ok((file, text))
                })
                .collect()
        };

        let mut note_types: Vec<PathBuf> = match fs::read_dir(self.root.join(parser::NOTE_TYPES_DIR)) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "toml") && p.is_file())
                .filter(|p| p.file_stem().is_some_and(|s| !s.to_string_lossy().starts_with('.')))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        note_types.sort();

        let cards_dir = self.root.join(CARDS_DIR);
        let mut decks = Vec::new();
        if cards_dir.is_dir() {
            collect_markdown(&cards_dir, &mut decks)?;
        }
        // By deck name, so a deck's own cards come before its subdecks'.
        decks.sort_by_key(|f| f.with_extension(""));

        let mut strays = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || [CARDS_DIR, MEDIA_DIR, parser::NOTE_TYPES_DIR].contains(&name.as_str()) {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                collect_markdown(&path, &mut strays)?;
            } else if path.extension().is_some_and(|e| e == "md") {
                strays.push(path);
            }
        }
        strays.sort();
        let strays = strays
            .into_iter()
            .filter_map(|file| fs::read_to_string(&file).ok().map(|text| (file, text)))
            .collect();

        let settings = match fs::read_to_string(self.root.join(settings::FILE)) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };

        Ok(Sources { settings, note_types: read(note_types)?, decks: read(decks)?, strays })
    }

    /// Move history saved under a card's id from before media links were resolved for
    /// ids to the card's id now. Runs at every load, so decks written since are caught.
    fn carry_over_history(&mut self) -> Result<()> {
        let moves: Vec<(String, String)> = self
            .cards
            .iter()
            .filter_map(|c| Some((c.parsed.old_id.clone()?, c.parsed.id.clone())))
            // An old id that is another card's id now is that card's history.
            .filter(|(old, _)| !self.index.contains_key(old))
            .collect();
        if !moves.is_empty() && self.store.move_history(&moves)? > 0 {
            self.states = self.store.states()?;
        }
        Ok(())
    }

    /// Markdown files with cards outside `cards`, as from before decks moved there, are
    /// left out; say so once, at the first of them.
    fn warn_about_stray_decks(&mut self, files: &[(PathBuf, String)]) {
        let stray: Vec<&PathBuf> = files
            .iter()
            .filter(|(file, text)| !parser::parse_deck(text, &self.note_types, &relative(&self.root, file)).cards.is_empty())
            .map(|(file, _)| file)
            .collect();
        if let Some(first) = stray.first() {
            let more = match stray.len() - 1 {
                0 => String::new(),
                1 => " and 1 more".into(),
                n => format!(" and {n} more"),
            };
            let message = format!("decks are read from the `{CARDS_DIR}` folder, so this file{more} are left out: move them into it");
            self.warnings.push(CollectionWarning { path: relative(&self.root, first), line: 0, message });
        }
    }

    /// The settings in `amgi.toml`; the defaults for those it leaves out, and for all of
    /// them if it has errors, which become a warning.
    fn load_settings_file(&mut self, text: Option<&str>) {
        let config = match text.map(settings::Settings::parse).transpose() {
            Ok(config) => config.unwrap_or_default(),
            Err(message) => {
                self.warnings.push(CollectionWarning { path: settings::FILE.into(), line: 0, message });
                settings::Settings::default()
            }
        };
        let text = text.unwrap_or_default();
        self.scheduler.desired_retention = config.study.retention.unwrap_or(Scheduler::default().desired_retention);
        self.rollover_secs = i64::from(config.study.day_starts_at.unwrap_or(DEFAULT_ROLLOVER_HOUR)) * 3600;
        self.reviews_per_day = config.review.per_day;
        self.leech_threshold = config.leech.lapses.unwrap_or(DEFAULT_LEECH_THRESHOLD);
        self.leech_suspend = config.leech.suspend.unwrap_or(false);
        self.new_per_day = config.new.per_day.unwrap_or(DEFAULT_NEW_PER_DAY);
        self.new_rules = config
            .new
            .rule
            .into_iter()
            .map(|r| NewRule {
                line: text[..r.cards.span().start].matches('\n').count() as u32 + 1,
                cards: r.cards.into_inner(),
                per_day: r.per_day,
            })
        .collect();
        let cap = self.new_per_day;
        for r in self.new_rules.iter().filter(|r| r.per_day > cap) {
            let message = format!(
                "the new-card rule for \"{}\" allows {} a day, more than the {cap} of new.per-day, so it has no effect",
                r.cards, r.per_day
            );
            self.warnings.push(CollectionWarning { path: settings::FILE.into(), line: r.line, message });
        }
    }

    /// Settings the database kept before `amgi.toml` did go to the file, each unless
    /// the file sets its own, and leave the database. A file with errors is left alone
    /// until it's fixed.
    fn move_settings_to_file(&mut self) -> Result<()> {
        type Format = fn(f64) -> String;
        let moves: [(&str, &str, &str, Format); 6] = [
            (RETENTION_KEY, "study", "retention", |v| format!("{}", (v.clamp(0.7, 0.99) * 100.0).round() / 100.0)),
            (ROLLOVER_HOUR_KEY, "study", "day-starts-at", |v| format!("{}", v.clamp(0.0, 23.0) as u8)),
            (NEW_PER_DAY_KEY, "new", "per-day", |v| format!("{}", v.clamp(0.0, u32::MAX.into()) as u32)),
            (REVIEWS_PER_DAY_KEY, "review", "per-day", |v| format!("{}", v.clamp(0.0, u32::MAX.into()) as u32)),
            (LEECH_THRESHOLD_KEY, "leech", "lapses", |v| format!("{}", v.clamp(1.0, u32::MAX.into()) as u32)),
            (LEECH_SUSPEND_KEY, "leech", "suspend", |v| format!("{}", v != 0.0)),
        ];
        let mut found = Vec::new();
        for (db_key, table, key, format) in moves {
            if let Some(value) = self.store.setting(db_key)? {
                found.push((db_key, table, key, format(value)));
            }
        }
        if found.is_empty() {
            return Ok(());
        }
        let path = self.root.join(settings::FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let Ok(config) = settings::Settings::parse(&text) else { return Ok(()) };
        let mut edited = text.clone();
        for (_, table, key, value) in found.iter().filter(|(_, table, key, _)| !config.has(table, key)) {
            edited = settings::set(&edited, table, key, value);
        }
        if edited != text {
            // Should the edit not read back, the file is left as it was.
            if settings::Settings::parse(&edited).is_err() {
                return Ok(());
            }
            fs::write(&path, edited)?;
            self.reload()?;
        }
        for (db_key, ..) in found {
            self.store.remove_setting(db_key)?;
        }
        Ok(())
    }

    /// Every `note-types/Name.toml`, as note type `Name`.
    fn load_note_types(&mut self, files: &[(PathBuf, String)]) {
        self.note_types = parser::NoteTypes::default();
        for (file, text) in files {
            let Some(name) = file.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            for w in self.note_types.insert(&name, text) {
                self.warnings.push(CollectionWarning { path: relative(&self.root, file), line: w.line, message: w.message });
            }
        }
    }

    /// Average time per card over the last `TIME_AVERAGE_DAYS` study days, today included.
    fn time_per_card(&self, ids: Option<&[String]>, now: i64) -> Result<CardTime> {
        let since = self.day_start(now) - (TIME_AVERAGE_DAYS - 1) * DAY_SECS;
        Ok(self.store.time_per_card(since, self.utc_offset_secs - self.rollover_secs, ids)?)
    }

    fn day_start(&self, now: i64) -> i64 {
        let local = now + self.utc_offset_secs - self.rollover_secs;
        local.div_euclid(DAY_SECS) * DAY_SECS + self.rollover_secs - self.utc_offset_secs
    }

    /// Whether a card is due at `now`. A failed card, back after minutes, is due at its
    /// time; a card on an interval of days is due for the whole study day it falls on.
    fn is_due(&self, state: &CardState, now: i64) -> bool {
        if state.interval_secs < DAY_SECS {
            state.due <= now
        } else {
            state.due < self.day_start(now) + DAY_SECS
        }
    }

    fn new_remaining(&self, now: i64) -> Result<u32> {
        let studied = self.store.new_cards_since(self.day_start(now), None)?;
        Ok(self.new_per_day.saturating_sub(studied))
    }

    /// What the new-card rules allow today: each counts the new cards it matches that
    /// were studied today, across the whole collection.
    fn rule_budget(&self, now: i64) -> Result<RuleBudget> {
        if self.new_rules.is_empty() {
            return Ok(RuleBudget::default());
        }
        let today = self.day_start(now);
        let types = self.note_types.list();
        let queries: Vec<Query> = self.new_rules.iter().map(|r| Query::parse_with(&r.cards, today, &types)).collect();
        let matching = |card: &Card| -> Vec<usize> {
            let summary = self.summary(card);
            (0..queries.len()).filter(|&r| queries[r].matches(&summary)).collect()
        };
        let mut left: Vec<u32> = self.new_rules.iter().map(|r| r.per_day).collect();
        for id in self.store.new_card_ids_since(today)? {
            if let Some(&i) = self.index.get(&id) {
                for r in matching(&self.cards[i]) {
                    left[r] = left[r].saturating_sub(1);
                }
            }
        }
        let rules = self
            .cards
            .iter()
            .filter(|c| !c.suspended() && !self.states.contains_key(&c.parsed.id))
            .filter_map(|c| {
                let rules = matching(c);
                (!rules.is_empty()).then(|| (c.parsed.id.clone(), rules))
            })
            .collect();
        Ok(RuleBudget { left, rules })
    }

    /// Reviews of due cards still allowed today, collection-wide.
    fn reviews_remaining(&self, now: i64) -> Result<u32> {
        let Some(limit) = self.reviews_per_day else { return Ok(u32::MAX) };
        let reviewed = self.store.due_reviews_since(self.day_start(now))?;
        Ok(limit.saturating_sub(reviewed))
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
        let new_remaining = self.new_remaining(now)?;
        let budget = self.rule_budget(now)?;
        let unseen = || self.cards.iter().filter(|c| !c.suspended() && !self.states.contains_key(&c.parsed.id));
        let reviews_remaining = self.reviews_remaining(now)?;
        // Count each deck's own cards, then add them to the deck and its parents:
        // there are far fewer decks than cards. `due` holds reviews until the limit
        // applies below.
        let mut own: HashMap<&str, (u32, u32, u32, u32)> = HashMap::new();
        for card in &self.cards {
            let counts = own.entry(&card.deck).or_default();
            counts.0 += 1;
            if card.suspended() {
                continue;
            }
            match self.states.get(&card.parsed.id) {
                Some(s) if s.interval_secs < DAY_SECS => counts.2 += 1,
                Some(s) if self.is_due(s, now) => counts.1 += 1,
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
                due: d.due.min(reviews_remaining),
                new: if budget.rules.is_empty() {
                    d.new.min(new_remaining)
                } else {
                    budget.available(unseen().filter(|c| contains(&d.name, &c.deck)), new_remaining)
                },
                note_type: self.deck_note_types.get(&d.name).cloned(),
                ..d
            })
            .collect())
    }

    fn cards(&self, scope: &StudyScope) -> Vec<CardSummary> {
        self.cards_where(scope, |_| true)
    }

    fn cards_where(&self, scope: &StudyScope, keep: impl Fn(&Card) -> bool) -> Vec<CardSummary> {
        self.in_scope(scope).filter(|card| keep(card)).map(|card| self.summary(card)).collect()
    }

    fn summary(&self, card: &Card) -> CardSummary {
        let state = self.states.get(&card.parsed.id);
        CardSummary {
            id: card.parsed.id.clone(),
            deck: card.deck.clone(),
            tags: card.parsed.tags.clone(),
            path: card.path.clone(),
            line: card.parsed.line,
            kind: card.parsed.kind,
            front: card.parsed.front.clone(),
            back: card.parsed.back.clone(),
            due: state.map(|s| s.due),
            interval: state.map(|s| s.interval_secs),
            reps: state.map_or(0, |s| s.reps),
            lapses: state.map_or(0, |s| s.lapses),
            suspended: card.suspended(),
            leech: state.is_some_and(|s| s.lapses >= self.leech_threshold),
            note_type: card.parsed.note.as_ref().map(|n| n.note_type.clone()),
            fields: card.parsed.note.as_ref().map(|n| n.values.clone()).unwrap_or_default(),
        }
    }

    fn next_card(&self, scope: &StudyScope, now: i64) -> Result<Option<StudyCard>> {
        let studied = || self.in_scope(scope).filter(|c| !c.suspended());
        let seen = || studied().filter_map(|c| self.states.get(&c.parsed.id).map(|s| (s, c)));
        // When nothing else is left, a failed card due back soon is shown early, but
        // never one from the next study day.
        let ahead = || {
            let limit = (now + LEARN_AHEAD_SECS).min(self.day_start(now) + DAY_SECS - 1);
            seen()
                .filter(|(s, _)| s.interval_secs < DAY_SECS && s.due <= limit)
                .min_by_key(|(s, _)| s.due)
                .map(|(_, c)| c)
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
        let review = || -> Result<Option<&Card>> {
            if self.reviews_remaining(now)? == 0 {
                return Ok(None);
            }
            Ok(seen()
                .filter(|(s, _)| self.is_due(s, now))
                .min_by_key(|(s, c)| {
                    let last_review = s.last_review.to_string();
                    (self.day_start(s.due), parser::stable_hash(&[&c.parsed.id, &last_review]))
                })
                .map(|(_, c)| c))
        };

        let card = match relearning.map_or_else(review, |c| Ok(Some(c)))? {
            Some(c) => Some(c),
            None if self.new_remaining(now)? > 0 => self.next_new_card(scope, now)?.or_else(ahead),
            None => ahead(),
        };
        card.map(|c| self.study_card(c, now)).transpose()
    }

    /// The next new card, with decks taking turns. From the top of the deck tree down,
    /// each step goes to the folder or file that has given the fewest new cards today,
    /// counting everything inside it; ties go by a hash of its name and the day. A file
    /// gives its cards in file order, skipping those a new-card rule holds back today.
    fn next_new_card(&self, scope: &StudyScope, now: i64) -> Result<Option<&Card>> {
        let budget = self.rule_budget(now)?;
        // Each deck's next new card.
        let mut decks = HashSet::new();
        let mut candidates: Vec<&Card> = self
            .in_scope(scope)
            .filter(|c| !c.suspended() && !self.states.contains_key(&c.parsed.id))
            .filter(|c| budget.allows(&c.parsed.id))
            .filter(|c| decks.insert(c.deck.as_str()))
            .collect();
        let mut today: HashMap<&str, u32> = HashMap::new();
        for id in self.store.new_card_ids_since(self.day_start(now))? {
            if let Some(&i) = self.index.get(&id) {
                *today.entry(self.cards[i].deck.as_str()).or_default() += 1;
            }
        }
        let day = self.day_start(now).to_string();

        // Every candidate is in the deck at `depth` components of its name. A step down
        // goes to a subdeck, with its own subdecks, or to that deck's own file, as
        // `a.md` is beside `a/`; the deck list makes `a` the parent of `a/b` too.
        let mut depth = 0;
        while candidates.len() > 1 {
            let mut steps: BTreeMap<(String, bool), Vec<&Card>> = BTreeMap::new();
            for card in candidates {
                let parts: Vec<&str> = card.deck.split('/').collect();
                let own_file = parts.len() == depth;
                let path = parts[..parts.len().min(depth + 1)].join("/");
                steps.entry((path, own_file)).or_default().push(card);
            }
            let given = |path: &str, own_file: bool| -> u32 {
                let counts = |deck: &str| if own_file { deck == path } else { contains(path, deck) };
                today.iter().filter(|(deck, _)| counts(deck)).map(|(_, n)| n).sum()
            };
            let (_, cards) = steps
                .into_iter()
                .min_by_key(|((path, own_file), _)| {
                    let kind = if *own_file { "file" } else { "deck" };
                    (given(path, *own_file), parser::stable_hash(&[path, kind, &day]))
                })
                .expect("there are candidates");
            candidates = cards;
            depth += 1;
        }
        Ok(candidates.first().copied())
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
            stage: match state {
                None => StudyStage::New,
                Some(s) if s.interval_secs < DAY_SECS => StudyStage::Learning,
                Some(_) => StudyStage::Due,
            },
            intervals: preview.iter().map(|o| o.interval_secs).collect(),
            typed: card.parsed.typed.as_ref().map(|t| t.answer.clone()),
            typed_field: card.parsed.typed.as_ref().and_then(|t| t.field.clone()),
            typed_hint: card.parsed.typed.as_ref().and_then(|t| t.hint.clone()),
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
                    taken_ms: r.taken_ms,
                })
                .collect(),
        }))
    }

    fn answer(&mut self, card_id: &str, rating: Rating, taken_ms: Option<u32>, now: i64) -> Result<Option<Leech>> {
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
            .record_review(card_id, now, rating, elapsed.into(), review_state, cap_taken(taken_ms), prev, outcome)?;
        self.states.insert(card_id.to_string(), state);
        self.answered.push(Answered { card_id: card_id.to_string(), review_id, prev, suspended: false });
        let lapsed = state.lapses > prev.map_or(0, |p| p.lapses);
        if !lapsed || !leech_flagged(state.lapses, self.leech_threshold) {
            return Ok(None);
        }
        let mut suspended = false;
        if self.leech_suspend && !self.cards[self.index[card_id]].suspended() {
            suspended = self.set_suspended(&[card_id.to_string()], true)? > 0;
            if let Some(last) = self.answered.last_mut() {
                last.suspended = suspended;
            }
        }
        Ok(Some(Leech { lapses: state.lapses, suspended }))
    }

    fn undo_answer(&mut self, now: i64) -> Result<Option<StudyCard>> {
        let Some(answered) = self.answered.pop() else { return Ok(None) };
        let Answered { card_id, review_id, prev, suspended } = answered;
        if let Err(e) = self.store.undo_review(&card_id, review_id, prev) {
            self.answered.push(Answered { card_id, review_id, prev, suspended });
            return Err(e.into());
        }
        match prev {
            Some(state) => self.states.insert(card_id.clone(), state),
            None => self.states.remove(&card_id),
        };
        if suspended && self.index.contains_key(&card_id) {
            self.set_suspended(std::slice::from_ref(&card_id), false)?;
        }
        match self.index.get(&card_id) {
            Some(&i) => self.study_card(&self.cards[i], now).map(Some),
            None => Ok(None),
        }
    }

    fn add_card(&mut self, card: &NewCard) -> Result<Vec<String>> {
        let (deck, block, ids) = self.card_block(card)?;
        self.append(&deck, &block, &ids)?;
        Ok(ids)
    }

    /// The deck, text and card ids `card` would be added as, or why it can't be.
    fn card_block(&self, card: &NewCard) -> Result<(String, String, Vec<String>)> {
        let invalid = |message: &str| AmgiError::InvalidCard { message: message.into() };
        let block = parser::write_card(card.kind, &card.front, &card.back, &card.tags)
            .map_err(|message| AmgiError::InvalidCard { message })?;
        let deck = card.deck.trim().trim_matches('/');
        if deck.is_empty() || deck.split('/').any(|part| part.trim().is_empty() || part.starts_with('.')) {
            return Err(invalid("Choose a deck for the card."));
        }
        let ids: Vec<String> = parser::parse_deck(&block, &self.note_types, &deck_file(deck)).cards.into_iter().map(|c| c.id).collect();
        if let Some(existing) = ids.iter().find_map(|id| self.index.get(id)) {
            let path = &self.cards[*existing].path;
            return Err(AmgiError::InvalidCard { message: format!("This card is already in {path}.") });
        }
        Ok((deck.to_string(), block, ids))
    }

    fn add_note(&mut self, note: &NewNote) -> Result<Vec<String>> {
        let (deck, block, ids) = self.note_block(note)?;
        self.append(&deck, &block, &ids)?;
        Ok(ids)
    }

    /// The deck, text and card ids `note` would be added as, or why it can't be.
    fn note_block(&self, note: &NewNote) -> Result<(String, String, Vec<String>)> {
        let deck = note.deck.trim().trim_matches('/');
        if deck.is_empty() || deck.split('/').any(|part| part.trim().is_empty() || part.starts_with('.')) {
            return Err(AmgiError::InvalidCard { message: "Choose a deck for the note.".into() });
        }
        let (block, ids) = parser::write_note(&note.note_type, &self.note_types, &note.values, &note.tags, &deck_file(deck))
            .map_err(|message| AmgiError::InvalidCard { message })?;
        if let Some(existing) = ids.iter().find_map(|id| self.index.get(id)) {
            let card = &self.cards[*existing];
            return Err(AmgiError::InvalidCard {
                message: format!("A card with the same front is already in {}:{}.", card.path, card.parsed.line),
            });
        }
        Ok((deck.to_string(), block, ids))
    }

    /// Append `block` to the end of `deck`'s file, which is created if needed, and
    /// check that it makes the cards `ids`; if not, the file is put back.
    fn append(&mut self, deck: &str, block: &str, ids: &[String]) -> Result<()> {
        let path = self.root.join(CARDS_DIR).join(format!("{deck}.md"));
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

    /// The ids of the cards in `scope`, for the review log; `None` for all of them.
    fn scope_ids(&self, scope: &StudyScope) -> Option<Vec<String>> {
        match scope {
            StudyScope::All => None,
            StudyScope::Deck { .. } => Some(self.in_scope(scope).map(|c| c.parsed.id.clone()).collect()),
        }
    }

    fn stats(&self, scope: &StudyScope, activity_days: u32, now: i64) -> Result<Stats> {
        let ids = self.scope_ids(scope);
        let ids = ids.as_deref();
        // Day `n` starts at `n * DAY_SECS - shift`, matching `day_start`.
        let shift = self.utc_offset_secs - self.rollover_secs;
        let today = (now + shift).div_euclid(DAY_SECS);
        let first = today - i64::from(activity_days.max(1)) + 1;

        // Every day studied, for the streak; the recent ones fill the activity heatmap.
        let per_day = self.store.reviews_per_day(0, shift, ids)?;
        let mut activity = vec![0; activity_days.max(1) as usize];
        let mut activity_secs = activity.clone();
        for &(day, count, ms) in &per_day {
            if let Ok(i) = usize::try_from(day - first) && i < activity.len() {
                activity[i] = count;
                activity_secs[i] = (ms / 1000) as u32;
            }
        }
        let time = self.time_per_card(ids, now)?;

        let studied: HashSet<i64> = per_day.into_iter().map(|(d, _, _)| d).collect();
        let from = if studied.contains(&today) { today } else { today - 1 };
        let streak = (0..).take_while(|n| studied.contains(&(from - n))).count() as u32;

        let mut forecast = vec![0; FORECAST_DAYS];
        let (mut new_cards, mut young_cards, mut mature_cards, mut suspended_cards) = (0, 0, 0, 0);
        for card in self.in_scope(scope) {
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

        let mut retention = RetentionTable::default();
        let year_start = (today - 364) * DAY_SECS - shift;
        for (day, mature, passed, failed) in self.store.recall_per_day(year_start, shift, MATURE_SECS, ids)? {
            let ago = today - day;
            let periods = [
                (ago == 0, &mut retention.today),
                (ago == 1, &mut retention.yesterday),
                (ago < 7, &mut retention.week),
                (ago < 30, &mut retention.month),
                (ago < 365, &mut retention.year),
            ];
            for (_, r) in periods.into_iter().filter(|(within, _)| *within) {
                r.add(mature, passed, failed);
            }
        }
        Ok(Stats {
            activity,
            activity_secs,
            secs_per_review: time.review_ms.map(|ms| ms / 1000.0),
            secs_per_due_card: time.seen_ms.or(time.new_ms).map(|ms| ms / 1000.0),
            activity_start: first * DAY_SECS - shift,
            forecast,
            streak,
            new_cards,
            young_cards,
            mature_cards,
            suspended_cards,
            retention,
        })
    }

    fn workload(
        &self,
        scope: &StudyScope,
        new_cards: u32,
        every_day: bool,
        keep_adding: bool,
        days: u32,
        now: i64,
    ) -> Result<Workload> {
        let shift = self.utc_offset_secs - self.rollover_secs;
        let today = (now + shift).div_euclid(DAY_SECS);
        let day = |t: i64| (t + shift).div_euclid(DAY_SECS) - today;
        let threshold = self.leech_threshold;
        let cards: Vec<PlannedCard> = self
            .in_scope(scope)
            .filter(|c| !c.suspended())
            .filter_map(|c| self.states.get(&c.parsed.id))
            .map(|s| PlannedCard {
                memory: s.memory,
                last_day: day(s.last_review),
                due_day: day(s.due),
                lapses: lapses_toward_leech(s.lapses, threshold),
            })
            .collect();
        // The collection's own habits once there are enough ratings to go by.
        let (first, passed) = self.store.rating_counts()?;
        let share = |counts: &[u32]| {
            let total: u32 = counts.iter().sum();
            (total >= MIN_RATINGS).then(|| counts.iter().map(|&n| n as f32 / total as f32).collect::<Vec<_>>())
        };
        let mut habits = RatingHabits::default();
        if let Some(p) = share(&first) {
            habits.first.copy_from_slice(&p);
        }
        if let Some(p) = share(&passed) {
            habits.passed.copy_from_slice(&p);
        }
        let suspend_after = self.leech_suspend.then_some(self.leech_threshold);
        let days = days.max(1) as usize;
        // Every day, until the cards not yet studied run out.
        let keep_adding = every_day && keep_adding;
        let pool = if keep_adding {
            new_cards.saturating_mul(days as u32)
        } else if every_day {
            self.in_scope(scope).filter(|c| !c.suspended() && !self.states.contains_key(&c.parsed.id)).count() as u32
        } else {
            new_cards
        };
        let mut left = pool;
        let new_studied = (0..days)
            .map(|_| {
                let n = new_cards.min(left);
                left -= n;
                n
            })
            .collect();
        // Cards don't affect each other without a review limit, so the new ones are
        // simulated apart and what they add stays clear of the others' noise.
        let reviews = self.scheduler.simulate_reviews(&cards, 0, 0, days, habits, suspend_after)?;
        let added_reviews = self.scheduler.simulate_reviews(&[], pool, new_cards, days, habits, suspend_after)?;
        let time = self.time_per_card(self.scope_ids(scope).as_deref(), now)?;
        Ok(Workload {
            reviews,
            added_reviews,
            new_cards,
            every_day,
            keep_adding,
            new_studied,
            secs_per_due_card: time.seen_ms.or(time.new_ms).map(|ms| ms / 1000.0),
            secs_per_new_card: time.new_ms.or(time.seen_ms).map(|ms| ms / 1000.0),
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
            let path = unused_path(&self.root.join(CARDS_DIR), deck);
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
            .flat_map(|(deck, markdown)| parser::parse_deck(markdown, &self.note_types, &deck_file(deck)).cards)
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
                    taken_ms,
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
                        taken_ms: cap_taken(taken_ms),
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

    fn today(&self, scope: &StudyScope, now: i64) -> Result<TodayStats> {
        let since = self.day_start(now);
        let ids = self.scope_ids(scope);
        let ids = ids.as_deref();
        let unseen = || self.in_scope(scope).filter(|c| !c.suspended() && !self.states.contains_key(&c.parsed.id));
        let (mut learning, mut due) = (0, 0);
        for state in self.in_scope(scope).filter(|c| !c.suspended()).filter_map(|c| self.states.get(&c.parsed.id)) {
            if state.interval_secs < DAY_SECS {
                learning += 1;
            } else {
                due += u32::from(self.is_due(state, now));
            }
        }
        let new_available = self.rule_budget(now)?.available(unseen(), self.new_remaining(now)?);
        let due_available = due.min(self.reviews_remaining(now)?);
        let time = self.time_per_card(ids, now)?;
        Ok(TodayStats {
            reviewed: self.store.reviews_since(since, ids)?,
            new_studied: self.store.new_cards_since(since, ids)?,
            new_available,
            due_available,
            learning,
            study_secs: (self.store.time_since(since, ids)? / 1000) as u32,
            secs_left: time_left(time, new_available, due_available, learning),
        })
    }
}

/// A review's time as the log keeps it: up to `MAX_ANSWER_MS`.
fn cap_taken(taken_ms: Option<u32>) -> Option<u32> {
    taken_ms.map(|ms| ms.min(MAX_ANSWER_MS))
}

/// About how long the cards left today take, in seconds: new and returning cards each
/// at their own average, which includes being failed and coming back the same day, and
/// learning cards, already past that, at the time of one review. A kind of card without
/// timed reviews takes the average of another.
fn time_left(time: CardTime, new: u32, due: u32, learning: u32) -> Option<u32> {
    let any = time.seen_ms.or(time.new_ms).or(time.review_ms)?;
    let ms = f64::from(new) * time.new_ms.unwrap_or(any)
        + f64::from(due) * time.seen_ms.unwrap_or(any)
        + f64::from(learning) * time.review_ms.unwrap_or(any);
    Some((ms / 1000.0).round() as u32)
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

/// Markdown files under `dir`, leaving out hidden ones.
fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if entry.file_type()?.is_dir() {
            collect_markdown(&path, out)?;
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

/// The file of `deck`, from the collection root.
fn deck_file(deck: &str) -> String {
    format!("{CARDS_DIR}/{deck}.md")
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
        // Decks go in `cards`; note types and media beside it.
        for (name, body) in files {
            let beside = [parser::NOTE_TYPES_DIR, MEDIA_DIR].iter().any(|d| name.starts_with(&format!("{d}/")));
            let path = if beside { dir.join(name) } else { dir.join(CARDS_DIR).join(name) };
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
        assert!(inner.warnings[0].message.contains("duplicate of cards/a.md:1"));

        let decks = inner.decks(T0).unwrap();
        let names: Vec<_> = decks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["a", "sub", "sub/b"]);
        assert_eq!((decks[1].total, decks[1].new), (2, 2));
        assert_eq!((decks[0].total, decks[0].new, decks[0].due), (2, 2, 0));

        // New cards come in file order.
        let card = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        assert_eq!(card.front, "1");
        assert_eq!(card.stage, StudyStage::New);
        assert_eq!(card.intervals.len(), 4);
        inner.answer(&card.id, Rating::Good, None, T0).unwrap();

        let card2 = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        assert_eq!(card2.front, "2");
        inner.answer(&card2.id, Rating::Again, None, T0).unwrap();

        // The failed card waits to come back, then is due.
        let decks = inner.decks(T0 + 60).unwrap();
        assert_eq!((decks[0].due, decks[0].learning, decks[0].new), (0, 1, 0));
        let decks = inner.decks(T0 + scheduler::AGAIN_DELAY_SECS).unwrap();
        assert_eq!((decks[0].due, decks[0].learning), (0, 1));

        // It is shown again within the learn-ahead window.
        let again = inner.next_card(&deck("a"), T0 + 60).unwrap().unwrap();
        assert_eq!(again.id, card2.id);
        assert_eq!(again.stage, StudyStage::Learning);

        // Two days later the first card is due; the failed one is still being learned.
        let decks = inner.decks(T0 + 2 * DAY_SECS).unwrap();
        assert_eq!((decks[0].due, decks[0].learning, decks[0].new), (1, 1, 0));
        let today = inner.today(&StudyScope::All, T0).unwrap();
        assert_eq!((today.new_studied, today.new_available), (2, 2));
        assert!(matches!(
            inner.answer("nope", Rating::Good, None, T0),
            Err(AmgiError::UnknownCard { .. })
        ));

        // Undo takes answers back latest first, leaving the cards as they were.
        let undone = inner.undo_answer(T0).unwrap().unwrap();
        assert_eq!(undone, card2);
        let undone = inner.undo_answer(T0).unwrap().unwrap();
        assert_eq!(undone, card);
        assert!(inner.states.is_empty());
        assert_eq!(inner.today(&StudyScope::All, T0).unwrap().new_studied, 0);
        assert!(inner.undo_answer(T0).unwrap().is_none());
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reviews_are_due_for_their_whole_study_day() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: -\n\nQ: 2\nA: -\n")]);
        let mut inner = c.lock();
        let ids: Vec<String> = inner.cards.iter().map(|c| c.parsed.id.clone()).collect();
        let hour = 3600;
        // UTC, so today's study day starts at 4 am.
        let today = inner.day_start(T0);
        let state = |due, interval_secs| CardState {
            memory: Memory { stability: 5.0, difficulty: 5.0 },
            due,
            last_review: due - interval_secs,
            reps: 1,
            lapses: 0,
            interval_secs,
        };
        let next = |inner: &Inner, now| inner.next_card(&StudyScope::All, now).unwrap().map(|c| c.id);
        let due = |inner: &Inner, now| inner.decks(now).unwrap()[0].due;
        inner.states.insert(ids[0].clone(), state(today + 20 * hour, 2 * DAY_SECS));
        inner.states.insert(ids[1].clone(), state(today + DAY_SECS + 5 * 60, 2 * DAY_SECS));

        // Due this evening: due from the start of the day.
        assert_eq!((due(&inner, today), next(&inner, today)), (1, Some(ids[0].clone())));
        inner.answer(&ids[0], Rating::Good, None, today).unwrap();
        // Due five minutes into tomorrow: not shown early, though nothing else is left.
        let late = today + DAY_SECS - 10 * 60;
        assert_eq!((due(&inner, late), next(&inner, late)), (0, None));
        assert_eq!(next(&inner, today + DAY_SECS), Some(ids[1].clone()));

        // A failed card is shown early when nothing else is left, but not past the
        // day's end.
        inner.answer(&ids[1], Rating::Again, None, today + 2 * hour).unwrap();
        assert_eq!(next(&inner, today + 2 * hour + 60), Some(ids[1].clone()));
        inner.answer(&ids[1], Rating::Again, None, late).unwrap();
        assert_eq!(next(&inner, late + 60), None);
        assert_eq!(next(&inner, late + scheduler::AGAIN_DELAY_SECS), Some(ids[1].clone()));

        // A later rollover hour moves the day's start.
        drop(inner);
        write_settings(&dir, &c, "[study]\nday-starts-at = 6\n");
        assert_eq!(c.lock().day_start(T0 + DAY_SECS) - today, DAY_SECS + 2 * hour);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_cards_take_turns() {
        let (dir, c) = collection(&[
            ("languages.md", "Q: l1\nA: -\n\nQ: l2\nA: -\n"),
            ("languages/korean.md", "Q: k1\nA: -\n\nQ: k2\nA: -\n\nQ: k3\nA: -\n\nQ: k4\nA: -\n"),
            ("languages/french.md", "Q: f1\nA: -\n\nQ: f2\nA: -\n"),
            ("misc.md", "Q: m1\nA: -\n\nQ: m2\nA: -\n\nQ: m3\nA: -\n\nQ: m4\nA: -\n\nQ: m5\nA: -\n"),
        ]);
        let mut inner = c.lock();
        inner.new_per_day = 100;
        let mut order = Vec::new();
        while let Some(card) = inner.next_card(&StudyScope::All, T0).unwrap() {
            // Asking again gives the same card.
            assert_eq!(inner.next_card(&StudyScope::All, T0).unwrap().unwrap().id, card.id);
            inner.answer(&card.id, Rating::Easy, None, T0).unwrap();
            order.push(card.front.trim().to_string());
        }
        assert_eq!(order.len(), 13);
        let picks = |prefix: char, n: usize| order[..n].iter().filter(|f| f.starts_with(prefix)).count();
        let count = |n: usize, decks: &str| decks.chars().map(|d| picks(d, n)).sum::<usize>();

        // `languages` and `misc` take turns while both have cards, `languages.md` counting
        // as part of `languages`; then languages' last cards are left.
        for n in 1..=10 {
            assert!(count(n, "lkf").abs_diff(count(n, "m")) <= 1, "{order:?}");
        }
        // Inside `languages`: its own file, `korean` and `french` take turns, until
        // the smaller ones run out.
        let languages: Vec<_> = order.iter().filter(|f| !f.starts_with('m')).collect();
        let firsts: Vec<char> = languages[..6].iter().map(|f| f.chars().next().unwrap()).collect();
        for chunk in firsts.chunks(3) {
            assert_eq!(chunk.iter().collect::<HashSet<_>>().len(), 3, "{order:?}");
        }
        // Each file keeps its own order.
        for deck in ['l', 'k', 'f', 'm'] {
            let cards: Vec<_> = order.iter().filter(|f| f.starts_with(deck)).collect();
            assert!(cards.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        }

        // Cards studied in one deck alone count toward its turn: after two korean cards,
        // studying everything starts away from `languages`.
        let (_dir2, c2) = collection(&[
            ("languages/korean.md", "Q: k1\nA: -\n\nQ: k2\nA: -\n\nQ: k3\nA: -\n"),
            ("misc.md", "Q: m1\nA: -\n\nQ: m2\nA: -\n\nQ: m3\nA: -\n"),
        ]);
        let mut inner2 = c2.lock();
        for _ in 0..2 {
            let card = inner2.next_card(&deck("languages/korean"), T0).unwrap().unwrap();
            inner2.answer(&card.id, Rating::Easy, None, T0).unwrap();
        }
        let fronts: Vec<String> = (0..2)
            .map(|_| {
                let card = inner2.next_card(&StudyScope::All, T0).unwrap().unwrap();
                inner2.answer(&card.id, Rating::Easy, None, T0).unwrap();
                card.front.trim().to_string()
            })
            .collect();
        assert_eq!(fronts, ["m1", "m2"]);

        // The next day, counts start over.
        let tomorrow = T0 + DAY_SECS;
        let card = inner2.next_card(&StudyScope::All, tomorrow).unwrap().unwrap();
        let misc_first = parser::stable_hash(&["misc", "deck", &inner2.day_start(tomorrow).to_string()])
            < parser::stable_hash(&["languages", "deck", &inner2.day_start(tomorrow).to_string()]);
        assert_eq!(card.front.trim(), if misc_first { "m3" } else { "k3" });
        drop(inner);
        drop(inner2);
        let _ = fs::remove_dir_all(dir);
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
        inner.answer(&ids[5], Rating::Good, None, T0 - DAY_SECS).unwrap();
        for id in &ids[..5] {
            inner.answer(id, Rating::Good, None, T0).unwrap();
        }
        let later = T0 + 60 * DAY_SECS;
        let next = |inner: &Inner, now| inner.next_card(&StudyScope::All, now).unwrap().unwrap().id;

        // The most overdue day first, then a fixed shuffle within the day, the same
        // each time it is asked.
        assert_eq!(next(&inner, later), ids[5]);
        inner.answer(&ids[5], Rating::Good, None, later).unwrap();
        let key = |id: &String| parser::stable_hash(&[id, &T0.to_string()]);
        let mut expected = ids[..5].to_vec();
        expected.sort_by_key(key);
        assert_eq!(next(&inner, later), expected[0]);
        assert_eq!(next(&inner, later), expected[0]);

        // A failed card comes back after its delay, ahead of the reviews still due.
        inner.answer(&expected[0], Rating::Again, None, later).unwrap();
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
        c.answer(id.clone(), Rating::Good, None).unwrap();
        let saved = dir.join("saved.sqlite");
        fs::copy(&db, &saved).unwrap();
        let reps = |c: &CardCollection| c.cards(StudyScope::All)[0].reps;
        assert_eq!(reps(&c), 1);
        assert!(c.warnings().is_empty());

        // Deleted: a new database, as after a restart, and new reviews land in it.
        fs::remove_file(&db).unwrap();
        assert_eq!(reps(&c), 0);
        assert!(c.warnings()[0].message.contains("deleted"));
        c.answer(id.clone(), Rating::Good, None).unwrap();
        assert_eq!(Store::open(&db).unwrap().reviews(&id).unwrap().len(), 1);

        // Replaced, as by a sync or restore: its history is used.
        c.answer(id.clone(), Rating::Good, None).unwrap();
        assert_eq!(reps(&c), 2);
        fs::rename(&saved, &db).unwrap();
        assert_eq!(reps(&c), 1);
        assert!(c.warnings()[0].message.contains("replaced"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lists_cards_with_state() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: one\n#x\n\nQ: 2\nA: two\n"), ("b.md", "Q: 3\nA: three\n")]);
        let mut inner = c.lock();
        let first = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        inner.answer(&first.id, Rating::Again, None, T0).unwrap();

        let cards = inner.cards(&deck("a"));
        let fronts: Vec<_> = cards.iter().map(|c| c.front.as_str()).collect();
        assert_eq!(fronts, ["1", "2"]);
        assert_eq!(cards[0].tags, ["x"]);
        assert_eq!((cards[0].reps, cards[0].due.is_some()), (1, true));
        assert_eq!((cards[1].reps, cards[1].due), (0, None));
        assert_eq!(inner.cards(&StudyScope::All).len(), 3);
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

        inner.answer(&id, Rating::Good, None, T0).unwrap();
        inner.answer(&id, Rating::Again, None, T0 + 3 * DAY_SECS).unwrap();
        let info = inner.card_info(&id, T0 + 3 * DAY_SECS).unwrap().unwrap();
        assert_eq!((info.reps, info.lapses), (2, 1));
        assert_eq!(info.reviews.iter().map(|r| r.rating).collect::<Vec<_>>(), [Rating::Good, Rating::Again]);
        assert!(info.reviews[0].was_new && info.reviews[1].elapsed_days == 3.0);
        assert_eq!(info.due, Some(T0 + 3 * DAY_SECS + scheduler::AGAIN_DELAY_SECS));
        assert!((info.retrievability.unwrap() - 1.0).abs() < 1e-6);
        let later = inner.card_info(&id, T0 + 10 * DAY_SECS).unwrap().unwrap();
        assert!(later.retrievability.unwrap() < 1.0);

        // Review stages: a lapse seen again the same day is relearned.
        inner.answer(&id, Rating::Good, None, T0 + 3 * DAY_SECS + 600).unwrap();
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
        assert_eq!(fs::read_to_string(dir.join("cards/a.md")).unwrap(), "+++\ntags = [\"lang\"]\n+++\nQ: one\nA: 1\n\nQ: two\nA: 2\n#numbers\n");
        let info = c.card_info(ids[0].clone()).unwrap().unwrap();
        assert_eq!(info.tags, ["lang", "numbers"]);

        // A new file in a new folder; a cloze makes a card per gap.
        let ids = c.add_card(new("sub/deck", CardKind::Cloze, "[Seoul] is in [Korea].")).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(fs::read_to_string(dir.join("cards/sub/deck.md")).unwrap(), "C: [Seoul] is in [Korea].\n#numbers\n");

        let err = |card| match c.add_card(card) {
            Err(AmgiError::InvalidCard { message }) => message,
            other => panic!("{other:?}"),
        };
        assert_eq!(err(new("b", CardKind::Basic, "two")), "This card is already in cards/a.md.");
        assert_eq!(err(new("../x", CardKind::Basic, "three")), "Choose a deck for the card.");
        // An unclosed code block would take the card in; the file is left as it was.
        assert!(err(new("b", CardKind::Basic, "three")).contains("unclosed code block"));
        assert_eq!(fs::read_to_string(dir.join("cards/b.md")).unwrap(), "```\nopen");
        drop(c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn forecast_counts_lapses_up_to_the_next_suspension() {
        // Threshold 8 flags lapses 8, 12, 16...
        let toward = |lapses| lapses_toward_leech(lapses, 8);
        assert_eq!(toward(0), 0);
        assert_eq!(toward(7), 7);
        assert_eq!(toward(8), 4);
        assert_eq!(toward(11), 7);
        assert_eq!(toward(12), 4);
        assert_eq!(lapses_toward_leech(5, 1), 0);
    }

    #[test]
    fn unsuspended_leeches_are_still_forecast() {
        let (dir, c) = collection(&[("a.md", "Q: capital of Peru\nA: Lima\n")]);
        let id = c.cards(StudyScope::All)[0].id.clone();
        write_settings(&dir, &c, "[leech]\nlapses = 1\nsuspend = true\n");
        let mut inner = c.lock();
        inner.answer(&id, Rating::Good, None, T0 - 2 * DAY_SECS).unwrap();
        // Forgotten yesterday, so a suspended leech due today; then unsuspended.
        assert!(inner.answer(&id, Rating::Again, None, T0 - DAY_SECS).unwrap().unwrap().suspended);
        inner.set_suspended(std::slice::from_ref(&id), false).unwrap();

        let w = inner.workload(&StudyScope::All, 0, false, false, 30, T0).unwrap();
        assert_eq!(w.reviews[0], 1.0);
        drop(inner);
        drop(c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workload_with_new_cards() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n")]);
        let mut inner = c.lock();
        let ids: Vec<String> = inner.cards.iter().map(|c| c.parsed.id.clone()).collect();
        // Failed yesterday, so due today; the others are new.
        inner.answer(&ids[0], Rating::Again, None, T0 - DAY_SECS).unwrap();

        let w = inner.workload(&StudyScope::All, 0, false, false, 30, T0).unwrap();
        assert_eq!(w.reviews.len(), 30);
        assert_eq!(w.reviews[0], 1.0);
        assert_eq!(w.added_reviews, vec![0.0; 30]);

        let w = inner.workload(&StudyScope::All, 20, false, false, 30, T0).unwrap();
        assert_eq!(w.new_cards, 20);
        assert_eq!(w.new_studied[..2], [20, 0]);
        assert_eq!(w.added_reviews[0], 0.0);
        assert!(w.added_reviews.iter().sum::<f64>() > 20.0);

        // One a day until the two new cards run out.
        let w = inner.workload(&StudyScope::All, 1, true, false, 30, T0).unwrap();
        assert_eq!(w.new_studied[..3], [1, 1, 0]);
        assert_eq!(w.added_reviews[0], 0.0);
        assert!(w.added_reviews.iter().sum::<f64>() > 0.0);
        // Or every day, as if more are added.
        let w = inner.workload(&StudyScope::All, 1, true, true, 30, T0).unwrap();
        assert_eq!(w.new_studied, vec![1; 30]);
        drop(inner);
        drop(c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stats_for_the_collection() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n")]);
        let mut inner = c.lock();
        let ids: Vec<String> = inner.cards.iter().map(|c| c.parsed.id.clone()).collect();
        // Study on three consecutive days, ending yesterday.
        inner.answer(&ids[0], Rating::Good, None, T0 - 2 * DAY_SECS).unwrap();
        inner.answer(&ids[1], Rating::Easy, None, T0 - DAY_SECS).unwrap();
        inner.answer(&ids[0], Rating::Again, None, T0 - DAY_SECS + 60).unwrap();

        let d = inner.stats(&StudyScope::All, 5, T0).unwrap();
        assert_eq!(d.activity, [0, 0, 1, 2, 0]);
        assert_eq!(inner.day_start(T0) - d.activity_start, 4 * DAY_SECS);
        assert_eq!(d.streak, 2);
        assert_eq!((d.new_cards, d.young_cards, d.mature_cards), (1, 2, 0));
        assert_eq!(d.forecast.len(), FORECAST_DAYS);
        // The failed card is due today; the Easy one later.
        assert_eq!(d.forecast[0], 1);
        assert_eq!(d.forecast.iter().sum::<u32>(), 2);
        // The Again came a day after the first review, which scheduled the card days ahead.
        let failed = Retention { young_failed: 1, ..Retention::default() };
        assert_eq!(d.retention.today, Retention::default());
        assert_eq!((d.retention.yesterday, d.retention.week, d.retention.year), (failed, failed, failed));

        inner.answer(&ids[2], Rating::Good, None, T0).unwrap();
        let d = inner.stats(&StudyScope::All, 5, T0).unwrap();
        assert_eq!((d.activity[4], d.streak), (1, 3));
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn time_spent_studying() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n\nQ: 4\nA: 4\n")]);
        let mut inner = c.lock();
        let ids: Vec<String> = inner.cards.iter().map(|c| c.parsed.id.clone()).collect();
        // Yesterday a new card took 15 s over two reviews.
        inner.answer(&ids[0], Rating::Again, Some(10_000), T0 - DAY_SECS).unwrap();
        inner.answer(&ids[0], Rating::Good, Some(5_000), T0 - DAY_SECS + 60).unwrap();
        // Today a new card left on screen counts a minute, a seen one 8 s, and one
        // not timed nothing.
        inner.answer(&ids[1], Rating::Good, Some(120_000), T0).unwrap();
        inner.answer(&ids[0], Rating::Good, Some(8_000), T0 + 120).unwrap();
        inner.answer(&ids[2], Rating::Good, None, T0 + 180).unwrap();

        let today = inner.today(&StudyScope::All, T0 + 200).unwrap();
        assert_eq!((today.reviewed, today.study_secs), (3, 68));
        // New cards at 37.5 s, the average of 15 s and 60 s; others at 8 s, or one
        // review's 20.75 s while learning.
        let left = f64::from(today.new_available) * 37.5
            + f64::from(today.due_available) * 8.0
            + f64::from(today.learning) * 20.75;
        assert_eq!(today.new_available, 1);
        assert_eq!(today.secs_left, Some(left.round() as u32));

        let d = inner.stats(&StudyScope::All, 3, T0 + 200).unwrap();
        assert_eq!(d.activity, [0, 2, 3]);
        assert_eq!(d.activity_secs, [0, 15, 68]);
        assert_eq!((d.secs_per_review, d.secs_per_due_card), (Some(20.75), Some(8.0)));

        let info = inner.card_info(&ids[1], T0 + 200).unwrap().unwrap();
        assert_eq!(info.reviews[0].taken_ms, Some(MAX_ANSWER_MS));
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn time_left_falls_back_to_other_averages() {
        let only_seen = CardTime { seen_ms: Some(10_000.0), ..CardTime::default() };
        assert_eq!(time_left(only_seen, 2, 3, 1), Some(60));
        assert_eq!(time_left(CardTime::default(), 2, 3, 1), None);
        let all = CardTime { new_ms: Some(30_000.0), seen_ms: Some(6_000.0), review_ms: Some(4_000.0) };
        assert_eq!(time_left(all, 1, 5, 2), Some(68));
    }

    #[test]
    fn retention_by_period_and_maturity() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n")]);
        let mut inner = c.lock();
        let id = inner.cards[0].parsed.id.clone();
        // Young until a review schedules the card 21 days or more ahead.
        inner.answer(&id, Rating::Easy, None, T0 - 300 * DAY_SECS).unwrap();
        inner.answer(&id, Rating::Easy, None, T0 - 200 * DAY_SECS).unwrap();
        inner.answer(&id, Rating::Easy, None, T0 - 20 * DAY_SECS).unwrap();
        inner.answer(&id, Rating::Again, None, T0 - 3 * DAY_SECS).unwrap();
        // Back after minutes, then days later: neither counts, as the Again scheduled
        // it minutes ahead.
        inner.answer(&id, Rating::Again, None, T0 - 3 * DAY_SECS + 600).unwrap();
        inner.answer(&id, Rating::Good, None, T0 - DAY_SECS).unwrap();
        inner.answer(&id, Rating::Good, None, T0).unwrap();

        let r = inner.stats(&StudyScope::All, 1, T0).unwrap().retention;
        let retention = |young_passed, young_failed, mature_passed, mature_failed| Retention {
            young_passed,
            young_failed,
            mature_passed,
            mature_failed,
        };
        assert_eq!(r.today, retention(1, 0, 0, 0));
        assert_eq!(r.yesterday, Retention::default());
        assert_eq!(r.week, retention(1, 0, 0, 1));
        assert_eq!(r.month, retention(1, 0, 1, 1));
        assert_eq!(r.year, retention(2, 0, 1, 1));
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stats_and_today_for_one_deck() {
        let (dir, c) = collection(&[
            ("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n"),
            ("a/sub.md", "Q: 3\nA: 3\n"),
            ("b.md", "Q: 4\nA: 4\n"),
        ]);
        let mut inner = c.lock();
        let id = |deck: &str| inner.cards.iter().find(|c| c.deck == deck).unwrap().parsed.id.clone();
        let (sub, b) = (id("a/sub"), id("b"));
        inner.answer(&sub, Rating::Good, None, T0 - DAY_SECS).unwrap();
        inner.answer(&b, Rating::Good, None, T0).unwrap();
        inner.answer(&b, Rating::Good, None, T0 + 60).unwrap();

        // A deck takes in its subdecks, and only their reviews.
        let a = StudyScope::Deck { name: "a".into() };
        let d = inner.stats(&a, 2, T0).unwrap();
        assert_eq!(d.activity, [1, 0]);
        assert_eq!((d.new_cards, d.young_cards, d.streak), (2, 1, 1));
        let today = inner.today(&a, T0).unwrap();
        assert_eq!((today.reviewed, today.new_studied, today.new_available), (0, 0, 2));

        let d = inner.stats(&StudyScope::Deck { name: "b".into() }, 2, T0).unwrap();
        assert_eq!((d.activity, d.new_cards, d.young_cards), (vec![0, 2], 0, 1));
        assert_eq!(inner.today(&StudyScope::All, T0).unwrap().reviewed, 2);
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
        let card = inner.next_card(&deck("en"), T0).unwrap().unwrap();
        assert_eq!(card.tags, ["Verbs", "lang"]);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_card_limit() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n")]);
        assert_eq!(c.lock().new_per_day, DEFAULT_NEW_PER_DAY);
        write_settings(&dir, &c, "[new]\nper-day = 1\n");
        let mut inner = c.lock();
        let card = inner.next_card(&StudyScope::All, T0).unwrap().unwrap();
        inner.answer(&card.id, Rating::Easy, None, T0).unwrap();
        assert!(inner.next_card(&StudyScope::All, T0).unwrap().is_none());
        assert_eq!(inner.decks(T0).unwrap()[0].new, 0);
        // Tomorrow the limit resets.
        assert!(inner.next_card(&StudyScope::All, T0 + DAY_SECS).unwrap().is_some());
        drop(inner);
        // Lowering the limit below what's been learned today leaves none.
        write_settings(&dir, &c, "[new]\nper-day = 0\n");
        assert_eq!(c.lock().decks(T0).unwrap()[0].new, 0);
        // Raising it lets more in the same day.
        write_settings(&dir, &c, "[new]\nper-day = 5\n");
        assert_eq!(c.lock().decks(T0).unwrap()[0].new, 2);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Write `amgi.toml` and read the collection again.
    fn write_settings(dir: &Path, c: &CardCollection, toml: &str) {
        fs::write(dir.join(settings::FILE), toml).unwrap();
        c.reload().unwrap();
    }

    #[test]
    fn settings_move_to_file() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n")]);
        let reopen = |c: CardCollection| {
            let store = c.inner.into_inner().unwrap().store;
            CardCollection::with_store(dir.clone(), store).unwrap()
        };
        let file = || fs::read_to_string(dir.join(settings::FILE)).unwrap_or_default();
        let db = |c: &CardCollection, key| c.lock().store.setting(key).unwrap();
        // Without any in the database, nothing is written, and the defaults hold.
        let c = reopen(c);
        assert_eq!(file(), "");
        assert_eq!((c.desired_retention(), c.rollover_hour(), c.reviews_per_day()), (0.9, 4, None));
        assert_eq!((c.leech_threshold(), c.suspends_leeches()), (DEFAULT_LEECH_THRESHOLD, false));

        // Those in the database go to the file, beside what's there, and leave the database.
        fs::write(dir.join(settings::FILE), "[lint]\ntags = false\n\n[[new.rule]]\ncards = \"a\"\nper-day = 1\n").unwrap();
        let saved = [
            (RETENTION_KEY, f64::from(0.85_f32)),
            (ROLLOVER_HOUR_KEY, 3.0),
            (NEW_PER_DAY_KEY, 7.0),
            (REVIEWS_PER_DAY_KEY, 150.0),
            (LEECH_THRESHOLD_KEY, 5.0),
            (LEECH_SUSPEND_KEY, 1.0),
        ];
        for (key, value) in saved {
            c.lock().store.set_setting(key, value).unwrap();
        }
        let c = reopen(c);
        assert_eq!(
            file(),
            "[lint]\ntags = false\n\n[new]\nper-day = 7\n\n[[new.rule]]\ncards = \"a\"\nper-day = 1\n\n\
             [study]\nretention = 0.85\nday-starts-at = 3\n\n[review]\nper-day = 150\n\n[leech]\nlapses = 5\nsuspend = true\n"
        );
        assert_eq!((c.desired_retention(), c.rollover_hour(), c.reviews_per_day()), (0.85, 3, Some(150)));
        let new_per_day = c.lock().new_per_day;
        assert_eq!((new_per_day, c.leech_threshold(), c.suspends_leeches()), (7, 5, true));
        assert!(saved.iter().all(|(key, _)| db(&c, key).is_none()));

        // The file's own settings win, and the database's are dropped.
        c.lock().store.set_setting(NEW_PER_DAY_KEY, 9.0).unwrap();
        let c = reopen(c);
        let new_per_day = c.lock().new_per_day;
        assert_eq!((new_per_day, db(&c, NEW_PER_DAY_KEY)), (7, None));
        // A file with errors is left alone, and the setting kept for later.
        fs::write(dir.join(settings::FILE), "[new\n").unwrap();
        c.lock().store.set_setting(NEW_PER_DAY_KEY, 9.0).unwrap();
        let c = reopen(c);
        assert_eq!((file().as_str(), db(&c, NEW_PER_DAY_KEY)), ("[new\n", Some(9.0)));
        assert_eq!(c.warnings().len(), 1);
        drop(c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_card_rules() {
        let (dir, c) = collection(&[
            ("a.md", "Q: a1\nA: -\n\nQ: a2\nA: -\n\nQ: a3\nA: -\n\nQ: a4\nA: -\n#hard\n"),
            ("b.md", "Q: b1\nA: -\n\nQ: b2\nA: -\n\nQ: b3\nA: -\n"),
        ]);
        let rules = "[new]\nper-day = 10\n\n[[new.rule]]\ncards = \"deck:a\"\nper-day = 2\n\n[[new.rule]]\ncards = \"deck:b\"\nper-day = 0\n";
        write_settings(&dir, &c, rules);
        let mut inner = c.lock();
        let new = |inner: &Inner| inner.decks(T0).unwrap().iter().map(|d| d.new).collect::<Vec<_>>();
        assert_eq!(new(&inner), [2, 0]);
        assert_eq!(inner.today(&StudyScope::All, T0).unwrap().new_available, 2);
        // Two from `a`, none from `b`, though the collection allows ten.
        let mut fronts = Vec::new();
        while let Some(card) = inner.next_card(&StudyScope::All, T0).unwrap() {
            inner.answer(&card.id, Rating::Easy, None, T0).unwrap();
            fronts.push(card.front.trim().to_string());
        }
        assert_eq!(fronts, ["a1", "a2"]);
        assert_eq!(new(&inner), [0, 0]);
        // Tomorrow the rules reset.
        assert_eq!(inner.next_card(&StudyScope::All, T0 + DAY_SECS).unwrap().unwrap().front.trim(), "a3");
        drop(inner);

        // A card matching two rules takes both; the stricter one holds it back while
        // the other's cards still come.
        let rules = "[new]\nper-day = 10\n\n[[new.rule]]\ncards = \"deck:a\"\nper-day = 3\n\n[[new.rule]]\ncards = \"#hard\"\nper-day = 0\n";
        write_settings(&dir, &c, rules);
        let mut inner = c.lock();
        let card = inner.next_card(&deck("a"), T0).unwrap().unwrap();
        assert_eq!(card.front.trim(), "a3");
        inner.answer(&card.id, Rating::Easy, None, T0).unwrap();
        assert!(inner.next_card(&deck("a"), T0).unwrap().is_none());
        drop(inner);

        // A rule above the daily limit is flagged, with its line; a broken file too.
        assert!(c.warnings().is_empty());
        write_settings(&dir, &c, &rules.replace("per-day = 10", "per-day = 2"));
        let w = c.warnings();
        assert_eq!((w.len(), w[0].path.as_str(), w[0].line), (1, "amgi.toml", 5));
        assert!(w[0].message.contains("\"deck:a\" allows 3 a day, more than the 2"), "{}", w[0].message);
        write_settings(&dir, &c, "[[new.rule]]\ncards = \"deck:a\"\n");
        let w = c.warnings();
        assert!(w.len() == 1 && w[0].message.contains("per-day"), "{w:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn review_limit() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n\nQ: 4\nA: 4\n")]);
        let mut inner = c.lock();
        for _ in 0..3 {
            let card = inner.next_card(&StudyScope::All, T0).unwrap().unwrap();
            inner.answer(&card.id, Rating::Good, None, T0).unwrap();
        }
        drop(inner);
        write_settings(&dir, &c, "[review]\nper-day = 1\n");
        assert_eq!(c.reviews_per_day(), Some(1));
        let mut inner = c.lock();
        let later = T0 + 100 * DAY_SECS;
        assert_eq!(inner.decks(later).unwrap()[0].due, 1);
        assert_eq!(inner.today(&StudyScope::All, later).unwrap().due_available, 1);

        // Forgetting the one review allowed brings it back, but no other review comes.
        let card = inner.next_card(&StudyScope::All, later).unwrap().unwrap();
        assert_eq!(card.stage, StudyStage::Due);
        inner.answer(&card.id, Rating::Again, None, later).unwrap();
        let back = later + 600;
        assert_eq!((inner.decks(back).unwrap()[0].due, inner.decks(back).unwrap()[0].learning), (0, 1));
        assert_eq!(inner.next_card(&StudyScope::All, back).unwrap().unwrap().id, card.id);
        inner.answer(&card.id, Rating::Good, None, back).unwrap();
        // New cards have their own limit.
        let new = inner.next_card(&StudyScope::All, back).unwrap().unwrap();
        assert_eq!(new.stage, StudyStage::New);
        inner.answer(&new.id, Rating::Easy, None, back).unwrap();
        drop(inner);
        write_settings(&dir, &c, "[new]\nper-day = 1\n\n[review]\nper-day = 1\n");
        let inner = c.lock();
        assert!(inner.next_card(&StudyScope::All, back).unwrap().is_none());
        assert_eq!(inner.today(&StudyScope::All, back).unwrap().due_available, 0);
        // Tomorrow the limit resets, and without one every due card comes.
        assert_eq!(inner.decks(later + DAY_SECS).unwrap()[0].due, 1);
        drop(inner);
        write_settings(&dir, &c, "[new]\nper-day = 1\n");
        assert_eq!(c.lock().decks(back).unwrap()[0].due, 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn leeches_are_flagged_at_the_threshold_then_every_half() {
        let flagged = |threshold| (1..=20).filter(|&n| leech_flagged(n, threshold)).collect::<Vec<_>>();
        assert_eq!(flagged(8), [8, 12, 16, 20]);
        assert_eq!(flagged(5), [5, 7, 9, 11, 13, 15, 17, 19]);
        assert_eq!(flagged(1), (1..=20).collect::<Vec<_>>());
    }

    #[test]
    fn leeches_are_reported_and_optionally_suspended() {
        let (dir, c) = collection(&[("a.md", "Q: capital of Peru\nA: Lima\n")]);
        let id = c.cards(StudyScope::All)[0].id.clone();
        write_settings(&dir, &c, "[leech]\nlapses = 2\n");
        let mut inner = c.lock();
        // Learning a card and forgetting it once doesn't make a leech.
        assert_eq!(inner.answer(&id, Rating::Good, None, T0).unwrap(), None);
        assert_eq!(inner.answer(&id, Rating::Again, None, T0 + DAY_SECS).unwrap(), None);
        // Passing it isn't a lapse, even past the threshold.
        assert_eq!(inner.answer(&id, Rating::Good, None, T0 + 2 * DAY_SECS).unwrap(), None);
        let leech = inner.answer(&id, Rating::Again, None, T0 + 3 * DAY_SECS).unwrap();
        assert_eq!(leech, Some(Leech { lapses: 2, suspended: false }));
        assert!(inner.cards(&StudyScope::All)[0].leech);
        drop(inner);

        // Suspending leeches tags the card in its file, and undoing the answer untags it.
        write_settings(&dir, &c, "[leech]\nlapses = 2\nsuspend = true\n");
        let mut inner = c.lock();
        let leech = inner.answer(&id, Rating::Again, None, T0 + 4 * DAY_SECS).unwrap();
        assert_eq!(leech, Some(Leech { lapses: 3, suspended: true }));
        let file = dir.join(CARDS_DIR).join("a.md");
        assert!(fs::read_to_string(&file).unwrap().contains("#suspended"));
        assert!(inner.cards(&StudyScope::All)[0].suspended);
        inner.undo_answer(T0 + 4 * DAY_SECS).unwrap().unwrap();
        assert!(!fs::read_to_string(&file).unwrap().contains("#suspended"));
        assert_eq!(inner.cards(&StudyScope::All)[0].lapses, 2);
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
        assert_eq!(inner.today(&StudyScope::All, T0).unwrap().new_available, 1);
        let card = inner.next_card(&StudyScope::All, T0).unwrap().unwrap();
        assert_eq!(card.front, "kept");
        inner.answer(&card.id, Rating::Good, None, T0).unwrap();
        assert!(inner.next_card(&StudyScope::All, T0).unwrap().is_none());
        let cards = inner.cards(&StudyScope::All);
        assert_eq!(cards.iter().map(|c| c.suspended).collect::<Vec<_>>(), [false, true, true]);
        assert_eq!(inner.stats(&StudyScope::All, 1, T0).unwrap().suspended_cards, 2);

        // Unsuspending edits the file; a front matter tag stays.
        assert_eq!(inner.set_suspended(&ids[1..], false).unwrap(), 1);
        assert_eq!(fs::read_to_string(dir.join("cards/a.md")).unwrap(), "Q: kept\nA: 1\n\nQ: paused\nA: 2\n");
        assert_eq!(inner.next_card(&StudyScope::All, T0).unwrap().unwrap().front, "paused");
        // And suspending puts the tag back, keeping the card's history.
        assert_eq!(inner.set_suspended(&ids[..1], true).unwrap(), 1);
        assert!(inner.states.contains_key(&ids[0]));
        assert_eq!(fs::read_to_string(dir.join("cards/a.md")).unwrap(), "Q: kept\nA: 1\n#suspended\n\nQ: paused\nA: 2\n");
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
        assert_eq!((start + 5 * 3600 - 4 * 3600).rem_euclid(DAY_SECS), 0);
        assert!(start <= 1_759_960_800 && 1_759_960_800 - start < DAY_SECS);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn history_follows_cards_to_their_resolved_media_ids() {
        let card = "Q: Which city? ![](../media/eiffel.jpg)\nA: Paris\n";
        let (dir, c) = collection(&[("Places.md", card)]);
        let mut inner = c.lock();
        let id = inner.cards[0].parsed.id.clone();
        let old = parser::parse(card).cards[0].id.clone();
        assert_ne!(id, old);
        // Reviewed by a version that hashed links as written.
        inner.answer(&id, Rating::Good, None, T0).unwrap();
        inner.store.move_history(&[(id.clone(), old.clone())]).unwrap();
        // And since then under the new id, before the old history moved over.
        inner.answer(&id, Rating::Good, None, T0 + DAY_SECS).unwrap();
        inner.states = inner.store.states().unwrap();

        // Loading merges both, keeping the latest state.
        inner.reload().unwrap();
        let moved = |inner: &Inner| {
            assert!(!inner.states.contains_key(&old));
            assert!(inner.store.reviews(&old).unwrap().is_empty());
            assert_eq!(inner.states[&id].last_review, T0 + DAY_SECS);
            assert_eq!(inner.store.reviews(&id).unwrap().len(), 2);
        };
        moved(&inner);

        // Moving the deck down a folder and fixing its link keeps it the same card.
        fs::remove_file(dir.join("cards/Places.md")).unwrap();
        fs::create_dir_all(dir.join("cards/Europe")).unwrap();
        fs::write(dir.join("cards/Europe/Places.md"), card.replace("../media", "../../media")).unwrap();
        inner.reload().unwrap();
        assert_eq!(inner.cards[0].parsed.id, id);
        moved(&inner);
        assert_eq!(inner.store.review_counts(&[id.clone(), old.clone()]).unwrap(), HashMap::from([(id.clone(), 2)]));
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
            taken_ms: None,
        };
        let import = || Import {
            decks: vec![("Default".into(), studied.into()), ("Lang/Verbs".into(), "Q: fresh\nA: no\n".into())],
            note_types: Vec::new(),
            histories: vec![(id.clone(), vec![review(0, 1), review(1, 3)]), (id.clone(), vec![review(0, 1)])],
        };
        let imported = c.import(import()).unwrap();
        assert_eq!(imported.files, ["cards/Default 2.md", "cards/Lang/Verbs.md"]);
        assert_eq!((imported.reviewed_cards, imported.existing_history, imported.new_cards), (1, 0, 1));
        assert_eq!(fs::read_to_string(dir.join("cards/Default.md")).unwrap(), "Q: mine\nA: kept\n");

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
                "cards/bad.md:4: note type `Bad` has errors, see `note-types/Bad.toml`",
            ]
        );
        // Listed cards carry their note's fields, and the note types say what they are.
        let cards = c.cards(StudyScope::All);
        let dos = cards.iter().find(|b| b.front == "dos").unwrap();
        assert_eq!((dos.note_type.as_deref(), dos.fields.clone()), (Some("Vocab"), vec!["dos".to_string(), "two".into()]));

        let inner = c.lock();
        let fronts: Vec<_> = inner.cards.iter().filter(|c| c.path == "cards/es.md").map(|c| c.parsed.front.as_str()).collect();
        assert_eq!(fronts, ["uno", "one", "dos", "two", "plain"]);
        let two = inner.cards[3].parsed.id.clone();
        drop(inner);
        // The deck's note type is the one most of its notes have.
        let decks = c.decks().unwrap();
        assert_eq!(decks.iter().find(|d| d.name == "es").unwrap().note_type.as_deref(), Some("Vocab"));
        assert_eq!(decks.iter().find(|d| d.name == "bad").unwrap().note_type, None);

        // Suspending a card suspends its note.
        assert_eq!(c.set_suspended(vec![two], true).unwrap(), 1);
        assert!(fs::read_to_string(dir.join("cards/es.md")).unwrap().contains("Word: dos\nMeaning: two\n#suspended\n\nQ: plain"));

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
        assert_eq!(c.import(import).unwrap().files, ["note-types/Other.toml", "cards/a.md", "cards/b.md"]);
        assert_eq!(c.import(other).unwrap().files, ["note-types/Vocab 2.toml", "cards/c.md"]);
        assert_eq!(fs::read_to_string(dir.join("cards/c.md")).unwrap(), "@Vocab 2\nWord: cinco\nMeaning: -\n");
        let inner = c.lock();
        let fronts: Vec<_> = inner.cards.iter().filter(|c| c.path == "cards/c.md").map(|c| c.parsed.front.as_str()).collect();
        assert_eq!(fronts, ["cinco"]);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decks_are_read_from_cards() {
        let (dir, c) = collection(&[("a.md", "Q: in cards\nA: yes\n")]);
        fs::write(dir.join("README.md"), "# My cards\n\nWhat I'm learning.\n").unwrap();
        fs::write(dir.join("old.md"), "Q: left out\nA: yes\n").unwrap();
        fs::create_dir_all(dir.join("lang")).unwrap();
        fs::write(dir.join("lang/ko.md"), "Q: also left out\nA: yes\n").unwrap();
        c.reload().unwrap();
        let fronts: Vec<_> = c.cards(StudyScope::All).into_iter().map(|c| (c.deck, c.path, c.front)).collect();
        assert_eq!(fronts, [("a".to_string(), "cards/a.md".to_string(), "in cards".to_string())]);
        // A README has no cards, so only the old decks are mentioned.
        let warnings: Vec<_> = c.warnings().into_iter().map(|w| format!("{}: {}", w.path, w.message)).collect();
        assert_eq!(warnings, ["lang/ko.md: decks are read from the `cards` folder, so this file and 1 more are left out: move them into it"]);
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
        assert_eq!(link, "![](<@/media/hola (1).mp3>)");
        // The same file again is shared; another with its name is kept beside it.
        assert_eq!(c.add_media("lang/es".into(), sound.to_string_lossy().into_owned()).unwrap(), link);
        fs::write(&sound, b"other").unwrap();
        assert_eq!(c.add_media("es".into(), sound.to_string_lossy().into_owned()).unwrap(), "![](<@/media/hola (1) 2.mp3>)");

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
            fs::read_to_string(dir.join("cards/lang/es.md")).unwrap(),
            "+++\nname = \"Spanish\"\n+++\n\n@Vocab\nWord: hola\nMeaning: hello\nAudio: ![](<@/media/hola (1).mp3>)\n#greeting\n"
        );
        assert!(matches!(c.add_note(note), Err(AmgiError::InvalidCard { message }) if message.contains("cards/lang/es.md:5")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_anki_package() {
        let (dir, c) = collection(&[("Default.md", "Q: mine\nA: kept\n")]);
        let package = dir.join("deck.apkg");
        anki::tests::write_package(&package);
        let summary = c.import_anki(package.to_string_lossy().into_owned()).unwrap();
        assert_eq!(summary.files, ["cards/Default 2.md", "cards/Lang/Kazakh-Verbs.md"]);
        assert_eq!((summary.cards, summary.reviewed_cards, summary.skipped_notes), (5, 3, 1));
        assert_eq!((summary.existing_history, summary.new_cards, summary.suspended_cards), (0, 2, 2));
        assert_eq!((summary.images, summary.missing_images), (1, 0));
        assert_eq!((summary.sounds, summary.missing_sounds), (1, 1));
        assert_eq!(fs::read(dir.join("media/hi.png")).unwrap(), b"png bytes");
        assert_eq!(fs::read(dir.join("media/paris.mp3")).unwrap(), b"mp3 bytes");

        let inner = c.lock();
        assert_eq!(fs::read_to_string(dir.join("cards/Default.md")).unwrap(), "Q: mine\nA: kept\n");
        let names: Vec<_> = inner.decks(T0).unwrap().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Default", "Default 2", "Lang", "Lang/Kazakh-Verbs"]);
        assert_eq!(inner.decks(T0).unwrap()[3].title, "Kazakh/Verbs");
        let names: Vec<_> = inner.tags().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["geo", "suspended"]);
        let card = inner.cards.iter().find(|c| c.parsed.front == "сәлем").unwrap();
        let state = inner.states[&card.parsed.id];
        // Due on the card's day in Anki (day 5), not 3 days after its last review as logged.
        // Due on Anki's day 5, whole days after the last review.
        assert_eq!((state.reps, state.last_review, state.due), (2, 1_700_086_400, 1_700_086_400 + 4 * DAY_SECS));
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
            taken_ms: None,
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
        let unscheduled = HistoryEntry::Review { time: T0, rating: Good, state: None, interval_secs: None, taken_ms: Some(90_000) };
        let (entries, state) = inner.replay(&[unscheduled]).unwrap().unwrap();
        let replayed = reviews(entries)[0];
        assert_eq!(state.due, T0 + replayed.interval_secs);
        // Imported times are capped like the app's own.
        assert_eq!(replayed.taken_ms, Some(MAX_ANSWER_MS));

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
    fn reload_parses_only_when_a_file_changed() {
        let (dir, c) = collection(&[("a.md", "Q: cat\nA: one\n")]);
        let fronts = |c: &CardCollection| c.cards(StudyScope::All).into_iter().map(|b| b.front).collect::<Vec<_>>();
        assert!(!c.reload().unwrap());

        // The same size, with the old modified date put back, still counts as changed.
        let path = dir.join("cards/a.md");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "Q: dog\nA: one\n").unwrap();
        fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
        assert!(c.reload().unwrap());
        assert_eq!(fronts(&c), ["dog"]);
        assert!(!c.reload().unwrap());

        // A note type added later is read, and the deck parsed again with it.
        fs::write(&path, "@Vocab\nWord: uno\nMeaning: one\n").unwrap();
        assert!(c.reload().unwrap());
        assert!(fronts(&c).is_empty());
        fs::create_dir_all(dir.join(parser::NOTE_TYPES_DIR)).unwrap();
        fs::write(dir.join(parser::NOTE_TYPES_DIR).join("Vocab.toml"), "fields = [\"Word\", \"Meaning\"]\n").unwrap();
        assert!(c.reload().unwrap());
        assert_eq!(fronts(&c), ["uno"]);

        // So is a deck left outside `cards`.
        fs::write(dir.join("old.md"), "Q: left\nA: behind\n").unwrap();
        assert!(c.reload().unwrap());
        assert!(c.warnings().iter().any(|w| w.path == "old.md"));
        assert!(!c.reload().unwrap());
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
