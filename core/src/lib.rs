//! amgi: plain-text flash cards scheduled with FSRS.
//!
//! A collection is a directory of Markdown files; each file is a deck. Review
//! state lives in `.amgi.sqlite` at the collection root.

pub mod anki;
pub mod parser;
pub mod scheduler;
pub mod store;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub use parser::CardKind;
pub use scheduler::Rating;
use parser::ParsedCard;
use scheduler::{Memory, ReviewState, Scheduler};
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
}

impl From<anki::Error> for AmgiError {
    fn from(e: anki::Error) -> Self {
        AmgiError::Import {
            message: e.to_string(),
        }
    }
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
pub struct DeckInfo {
    /// File path relative to the collection root, without the `.md` extension, or a
    /// folder. A deck contains the decks below it: `lang` holds `lang.md` and the `lang` folder.
    pub name: String,
    /// The `name` from the deck's front matter, or the last part of `name`.
    pub title: String,
    pub total: u32,
    pub due: u32,
    /// New cards available today, capped by the daily new-card limit.
    pub new: u32,
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
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
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
}

/// Everything known about one card: its content, schedule, memory and review history.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
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
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
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

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CollectionWarning {
    pub path: String,
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodayStats {
    pub reviewed: u32,
    pub new_studied: u32,
    /// New cards still available today across the collection, capped by the daily limit.
    pub new_available: u32,
}

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

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
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
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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
    states: HashMap<String, CardState>,
    warnings: Vec<CollectionWarning>,
    new_per_day: u32,
    utc_offset_secs: i64,
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

    /// Every tag in the collection, sorted, in the spelling first seen.
    pub fn tags(&self) -> Vec<String> {
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

    pub fn today(&self) -> Result<TodayStats> {
        self.lock().today(unix_now())
    }

    /// Collection-wide statistics, with `activity_days` days of review history.
    pub fn dashboard(&self, activity_days: u32) -> Result<Dashboard> {
        self.lock().dashboard(activity_days, unix_now())
    }

    pub fn warnings(&self) -> Vec<CollectionWarning> {
        self.lock().warnings.clone()
    }

    /// Import an Anki package (`.apkg` or `.colpkg`) as Markdown decks in this
    /// collection, keeping review history. Images and sounds go to a `media` folder
    /// at the root. Existing files are never overwritten.
    pub fn import_anki(&self, path: String) -> Result<ImportSummary> {
        let package = anki::Package::read(Path::new(&path))?;
        let conversion = package.convert();
        let root = self.lock().root.clone();
        let media_dir = root.join(MEDIA_DIR);
        let images = package.write_media(&conversion.media.images, &media_dir)?;
        let sounds = package.write_media(&conversion.media.sounds, &media_dir)?;
        let mut summary = self.lock().import_anki(conversion)?;
        summary.missing_images -= images;
        summary.images = images;
        summary.missing_sounds -= sounds;
        summary.sounds = sounds;
        Ok(summary)
    }
}

impl CardCollection {
    fn with_store(root: PathBuf, store: Store) -> Result<Self> {
        let states = store.states()?;
        let mut inner = Inner {
            root,
            store,
            scheduler: Scheduler::default(),
            cards: Vec::new(),
            index: HashMap::new(),
            deck_titles: HashMap::new(),
            states,
            warnings: Vec::new(),
            new_per_day: 20,
            utc_offset_secs: 0,
        };
        inner.reload()?;
        Ok(CardCollection {
            inner: Mutex::new(inner),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic mid-review leaves no partial state worth discarding: SQLite writes are transactional.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Inner {
    fn reload(&mut self) -> Result<()> {
        let mut files = Vec::new();
        collect_markdown(&self.root, &mut files)?;
        // By deck name, so a deck's own cards come before its subdecks'.
        files.sort_by_key(|f| f.with_extension(""));

        self.cards.clear();
        self.index.clear();
        self.deck_titles.clear();
        self.warnings.clear();
        for file in files {
            let rel = relative(&self.root, &file);
            let deck = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
            let result = parser::parse(&fs::read_to_string(&file)?);
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

    fn tags(&self) -> Vec<String> {
        let mut tags: Vec<&String> = Vec::new();
        let mut seen = HashSet::new();
        for tag in self.cards.iter().flat_map(|c| &c.parsed.tags) {
            if seen.insert(tag.to_lowercase()) {
                tags.push(tag);
            }
        }
        tags.sort_by_key(|t| t.to_lowercase());
        tags.into_iter().cloned().collect()
    }

    fn decks(&self, now: i64) -> Result<Vec<DeckInfo>> {
        let remaining = self.new_remaining(now)?;
        // Count each deck's own cards, then add them to the deck and its parents:
        // there are far fewer decks than cards.
        let mut own: HashMap<&str, (u32, u32, u32)> = HashMap::new();
        for card in &self.cards {
            let counts = own.entry(&card.deck).or_default();
            counts.0 += 1;
            if card.suspended() {
                continue;
            }
            match self.states.get(&card.parsed.id) {
                Some(s) if s.due <= now => counts.1 += 1,
                Some(_) => {}
                None => counts.2 += 1,
            }
        }
        // Keyed by path components, so parents sort before their children.
        let mut decks: BTreeMap<Vec<&str>, DeckInfo> = BTreeMap::new();
        for (name, (total, due, new)) in own {
            let parts: Vec<&str> = name.split('/').collect();
            for depth in 1..=parts.len() {
                let deck = decks.entry(parts[..depth].to_vec()).or_insert_with(|| DeckInfo {
                    name: parts[..depth].join("/"),
                    title: String::new(),
                    total: 0,
                    due: 0,
                    new: 0,
                });
                deck.total += total;
                deck.due += due;
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
                ..d
            })
            .collect())
    }

    fn browse(&self, scope: &StudyScope) -> Vec<BrowseCard> {
        self.in_scope(scope)
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
                }
            })
            .collect()
    }

    fn next_card(&self, scope: &StudyScope, now: i64) -> Result<Option<StudyCard>> {
        let studied = || self.in_scope(scope).filter(|c| !c.suspended());
        let earliest_due_before = |limit: i64| {
            studied()
                .filter_map(|c| self.states.get(&c.parsed.id).map(|s| (s.due, c)))
                .filter(|(due, _)| *due <= limit)
                .min_by_key(|(due, _)| *due)
                .map(|(_, c)| c)
        };

        let card = match earliest_due_before(now) {
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
        let state = self
            .store
            .record_review(card_id, now, rating, elapsed.into(), review_state, prev, outcome)?;
        self.states.insert(card_id.to_string(), state);
        Ok(())
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
                if let Some(edited) = parser::set_card_tag(&source, line, parser::SUSPENDED_TAG, suspended) {
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

        let mut activity = vec![0; activity_days.max(1) as usize];
        for (day, count) in self.store.reviews_per_day(first * DAY_SECS - shift, shift)? {
            if let Some(slot) = usize::try_from(day - first).ok().and_then(|i| activity.get_mut(i)) {
                *slot = count;
            }
        }

        let studied: HashSet<i64> = self.store.reviews_per_day(0, shift)?.into_iter().map(|(d, _)| d).collect();
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

    fn import_anki(&mut self, conversion: anki::Conversion) -> Result<ImportSummary> {
        let mut files = Vec::new();
        for (deck, markdown) in &conversion.decks {
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
        for (id, log) in &conversion.histories {
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
        let written: HashSet<String> = conversion
            .decks
            .iter()
            .flat_map(|(_, markdown)| parser::parse(markdown).cards)
            .map(|card| card.id)
            .collect();
        let studied = written.iter().filter(|id| self.states.contains_key(*id)).count() as u32;

        Ok(ImportSummary {
            files,
            notes: conversion.notes,
            cards: conversion.cards,
            reviewed_cards,
            existing_history: studied.saturating_sub(reviewed_cards),
            new_cards: written.len() as u32 - studied,
            skipped_notes: conversion.skipped_notes,
            suspended_cards: conversion.suspended_cards,
            images: 0,
            missing_images: conversion.media.images.len() as u32,
            sounds: 0,
            missing_sounds: conversion.media.sounds.len() as u32,
        })
    }

    /// Rebuild a card's memory state by replaying its Anki reviews through the
    /// scheduler. Due dates follow Anki's intervals so the existing schedule is kept.
    /// Filtered-deck reviews with rescheduling off left Anki's schedule alone, so they
    /// are skipped. `None` if the card ends up new.
    fn replay(&self, log: &[anki::Revlog]) -> Result<Option<(Vec<LogEntry>, CardState)>> {
        let mut entries = Vec::new();
        let mut state: Option<CardState> = None;
        let mut last_review_state = ReviewState::New;
        for entry in log {
            let rating = match entry.ease {
                1 => Some(Rating::Again),
                2 => Some(Rating::Hard),
                3 => Some(Rating::Good),
                4 => Some(Rating::Easy),
                _ => None,
            };
            match (rating, entry.kind) {
                (Some(_), 3) if entry.factor == 0 => {}
                (Some(rating), 0..=3) => {
                    let (memory, elapsed) = self.elapsed(state.as_ref(), entry.time);
                    // A history that starts mid-way has no New review, which tells the
                    // optimizer to leave it out.
                    let review_state = match (entry.kind, state) {
                        (0, None) => ReviewState::New,
                        (0, Some(_)) => ReviewState::Learning,
                        (1, _) => ReviewState::Review,
                        (2, _) => ReviewState::Relearning,
                        (_, None) => ReviewState::Review,
                        (_, Some(_)) => ReviewState::after(last_review_state, elapsed),
                    };
                    last_review_state = review_state;
                    let outcome = self.scheduler.schedule(memory, elapsed, rating)?;
                    let interval_secs = match entry.interval {
                        0 => outcome.interval_secs,
                        _ => entry.interval_secs(),
                    };
                    entries.push(LogEntry::Review(LoggedReview {
                        reviewed_at: entry.time,
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
                        due: entry.time + interval_secs,
                        last_review: entry.time,
                        reps: state.map_or(0, |s| s.reps) + 1,
                        lapses: state.map_or(0, |s| s.lapses) + lapse,
                    });
                }
                // Manual entries: "Forget" resets the card, "Set Due Date" and FSRS
                // rescheduling move the due date.
                (None, 4) if entry.interval == 0 => {
                    if state.take().is_some() {
                        entries.push(LogEntry::Reset(entry.time));
                    }
                }
                (None, 4 | 5) => {
                    if let Some(s) = state.as_mut() {
                        s.due = entry.time + entry.interval_secs();
                    }
                }
                _ => {}
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

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_dir() {
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

        // The failed card is shown again within the learn-ahead window.
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
        drop(inner);
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
        assert_eq!(inner.tags(), ["lang", "Verbs"]);
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
        assert_eq!(inner.tags(), ["geo", "suspended"]);
        let card = inner.cards.iter().find(|c| c.parsed.front == "сәлем").unwrap();
        let state = inner.states[&card.parsed.id];
        assert_eq!((state.reps, state.last_review, state.due), (2, 1_700_086_400, 1_700_086_400 + 3 * DAY_SECS));
        assert_eq!(inner.store.state(&card.parsed.id).unwrap(), Some(state));
        drop(inner);

        // Importing again keeps the history already here and says so.
        let again = c.import_anki(package.to_string_lossy().into_owned()).unwrap();
        assert_eq!((again.reviewed_cards, again.existing_history, again.new_cards), (0, 3, 2));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replay_follows_anki_schedule() {
        let (dir, c) = collection(&[("a.md", "")]);
        let inner = c.lock();
        let entry = |day: i64, ease, kind, interval| anki::Revlog {
            time: T0 + day * DAY_SECS,
            ease,
            kind,
            interval,
            factor: 2500,
        };
        let reviews = |entries: Vec<LogEntry>| -> Vec<LoggedReview> {
            entries.into_iter().filter_map(|e| if let LogEntry::Review(r) = e { Some(r) } else { None }).collect()
        };
        let log = [entry(0, 1, 0, -600), entry(0, 3, 0, 1), entry(1, 3, 1, 4), entry(5, 1, 1, -600), entry(5, 3, 2, 2)];
        let (entries, state) = inner.replay(&log).unwrap().unwrap();
        let logged = reviews(entries);
        assert_eq!(logged.len(), 5);
        assert!(logged[0].was_new && !logged[1].was_new);
        assert_eq!(logged.iter().map(|r| r.elapsed_days).collect::<Vec<_>>(), [0.0, 0.0, 1.0, 4.0, 0.0]);
        use ReviewState::*;
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

        // Filtered-deck reviews count unless rescheduling was off.
        let filtered = entry(3, 3, 3, 5);
        let practice = anki::Revlog { factor: 0, ..filtered };
        let (entries, state) = inner.replay(&[log[1], filtered, practice]).unwrap().unwrap();
        assert_eq!(reviews(entries).iter().map(|r| r.state).collect::<Vec<_>>(), [Some(New), Some(Review)]);
        assert_eq!((state.reps, state.due), (2, T0 + 8 * DAY_SECS));

        // Set Due Date moves the card; Forget makes it new again and is logged.
        let (_, moved) = inner.replay(&[log[1], entry(1, 0, 4, 10)]).unwrap().unwrap();
        assert_eq!(moved.due, T0 + 11 * DAY_SECS);
        assert!(inner.replay(&[log[1], entry(1, 0, 4, 0)]).unwrap().is_none());
        let (entries, state) = inner.replay(&[log[1], entry(1, 0, 4, 0), entry(2, 3, 0, 1)]).unwrap().unwrap();
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
