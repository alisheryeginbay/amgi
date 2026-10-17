//! SQLite persistence. Card content lives in Markdown; the database only holds
//! scheduling state and the review log, keyed by content-derived card ids.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::SavedView;
use crate::scheduler::{Memory, Outcome, Rating};

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE card_state (
    card_id     TEXT PRIMARY KEY,
    stability   REAL NOT NULL,
    difficulty  REAL NOT NULL,
    due         INTEGER NOT NULL,
    last_review INTEGER NOT NULL,
    reps        INTEGER NOT NULL,
    lapses      INTEGER NOT NULL
);
CREATE TABLE review_log (
    id            INTEGER PRIMARY KEY,
    card_id       TEXT NOT NULL,
    reviewed_at   INTEGER NOT NULL,
    rating        INTEGER NOT NULL,
    elapsed_days  REAL NOT NULL,
    stability     REAL NOT NULL,
    difficulty    REAL NOT NULL,
    interval_secs INTEGER NOT NULL,
    was_new       INTEGER NOT NULL
);
CREATE INDEX review_log_card ON review_log (card_id, reviewed_at);
CREATE INDEX review_log_time ON review_log (reviewed_at);
"#, r#"
CREATE TABLE tag_icon (
    tag  TEXT PRIMARY KEY,
    icon TEXT NOT NULL
);
"#, r#"
CREATE TABLE saved_view (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL,
    query TEXT NOT NULL,
    icon  TEXT
);
INSERT INTO saved_view (name, query, icon) VALUES ('Leeches', 'is:leech', '🐞');
"#];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardState {
    pub memory: Memory,
    /// Unix seconds.
    pub due: i64,
    pub last_review: i64,
    pub reps: u32,
    pub lapses: u32,
}

/// One row of the review log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoggedReview {
    pub reviewed_at: i64,
    pub rating: Rating,
    pub elapsed_days: f64,
    /// Memory state after the review.
    pub memory: Memory,
    pub interval_secs: i64,
    pub was_new: bool,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        // Rollback journal instead of WAL: the database stays a single file, which is
        // safer in synced folders, and one small write per review needs no concurrency.
        // Also converts databases created in WAL mode by earlier versions.
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(Store { conn })
    }

    pub fn states(&self) -> rusqlite::Result<HashMap<String, CardState>> {
        let mut stmt = self.conn.prepare(
            "SELECT card_id, stability, difficulty, due, last_review, reps, lapses FROM card_state",
        )?;
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                CardState {
                    memory: Memory {
                        stability: r.get(1)?,
                        difficulty: r.get(2)?,
                    },
                    due: r.get(3)?,
                    last_review: r.get(4)?,
                    reps: r.get(5)?,
                    lapses: r.get(6)?,
                },
            ))
        })?
        .collect()
    }

    pub fn state(&self, card_id: &str) -> rusqlite::Result<Option<CardState>> {
        self.conn
            .query_row(
                "SELECT stability, difficulty, due, last_review, reps, lapses
                 FROM card_state WHERE card_id = ?1",
                [card_id],
                |r| {
                    Ok(CardState {
                        memory: Memory {
                            stability: r.get(0)?,
                            difficulty: r.get(1)?,
                        },
                        due: r.get(2)?,
                        last_review: r.get(3)?,
                        reps: r.get(4)?,
                        lapses: r.get(5)?,
                    })
                },
            )
            .optional()
    }

    /// A card's review log, oldest first. Rows with an unknown rating are skipped.
    pub fn reviews(&self, card_id: &str) -> rusqlite::Result<Vec<LoggedReview>> {
        let mut stmt = self.conn.prepare(
            "SELECT reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new
             FROM review_log WHERE card_id = ?1 ORDER BY reviewed_at, id",
        )?;
        let rows = stmt.query_map([card_id], |r| {
            let Some(rating) = Rating::from_number(r.get(1)?) else { return Ok(None) };
            Ok(Some(LoggedReview {
                reviewed_at: r.get(0)?,
                rating,
                elapsed_days: r.get(2)?,
                memory: Memory {
                    stability: r.get(3)?,
                    difficulty: r.get(4)?,
                },
                interval_secs: r.get(5)?,
                was_new: r.get(6)?,
            }))
        })?;
        rows.filter_map(Result::transpose).collect()
    }

    /// Persist a review: append to the log and upsert the card's state atomically.
    pub fn record_review(
        &mut self,
        card_id: &str,
        now: i64,
        rating: Rating,
        elapsed_days: f64,
        prev: Option<CardState>,
        outcome: Outcome,
    ) -> rusqlite::Result<CardState> {
        let lapse = u32::from(prev.is_some() && rating == Rating::Again);
        let state = CardState {
            memory: outcome.memory,
            due: now + outcome.interval_secs,
            last_review: now,
            reps: prev.map_or(0, |p| p.reps) + 1,
            lapses: prev.map_or(0, |p| p.lapses) + lapse,
        };
        let tx = self.conn.transaction()?;
        insert_review(
            &tx,
            card_id,
            &LoggedReview {
                reviewed_at: now,
                rating,
                elapsed_days,
                memory: state.memory,
                interval_secs: outcome.interval_secs,
                was_new: prev.is_none(),
            },
        )?;
        upsert_state(&tx, card_id, &state)?;
        tx.commit()?;
        Ok(state)
    }

    /// Persist the history and resulting state of many cards in one transaction.
    pub fn import(&mut self, cards: &[(String, Vec<LoggedReview>, CardState)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for (card_id, reviews, state) in cards {
            for review in reviews {
                insert_review(&tx, card_id, review)?;
            }
            upsert_state(&tx, card_id, state)?;
        }
        tx.commit()
    }

    /// Number of new cards first reviewed at or after `since`.
    pub fn new_cards_since(&self, since: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM review_log WHERE was_new AND reviewed_at >= ?1",
            [since],
            |r| r.get(0),
        )
    }

    /// Review counts per day at or after `since`, where day `n` starts at
    /// `n * 86400 - shift` seconds. Days without reviews are absent.
    pub fn reviews_per_day(&self, since: i64, shift: i64) -> rusqlite::Result<Vec<(i64, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT (reviewed_at + ?2) / 86400 AS day, COUNT(*) FROM review_log
             WHERE reviewed_at >= ?1 GROUP BY day ORDER BY day",
        )?;
        stmt.query_map([since, shift], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    /// Reviews of already-seen cards at least a day after the previous review, at or
    /// after `since`, and how many of them were not rated Again.
    pub fn recall_since(&self, since: i64) -> rusqlite::Result<(u32, u32)> {
        self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(rating > 1), 0) FROM review_log
             WHERE NOT was_new AND elapsed_days >= 1 AND reviewed_at >= ?1",
            [since],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    pub fn reviews_since(&self, since: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM review_log WHERE reviewed_at >= ?1",
            [since],
            |r| r.get(0),
        )
    }

    /// Lowercased tag → icon.
    pub fn tag_icons(&self) -> rusqlite::Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT tag, icon FROM tag_icon")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    /// `None` removes the icon. `tag` is stored lowercased.
    pub fn set_tag_icon(&self, tag: &str, icon: Option<&str>) -> rusqlite::Result<()> {
        let tag = tag.to_lowercase();
        match icon {
            Some(icon) => self.conn.execute(
                "INSERT INTO tag_icon (tag, icon) VALUES (?1, ?2)
                 ON CONFLICT (tag) DO UPDATE SET icon = excluded.icon",
                params![tag, icon],
            )?,
            None => self.conn.execute("DELETE FROM tag_icon WHERE tag = ?1", [tag])?,
        };
        Ok(())
    }

    pub fn saved_views(&self) -> rusqlite::Result<Vec<SavedView>> {
        let mut stmt = self.conn.prepare("SELECT id, name, query, icon FROM saved_view ORDER BY id")?;
        stmt.query_map([], |r| Ok(SavedView { id: r.get(0)?, name: r.get(1)?, query: r.get(2)?, icon: r.get(3)? }))?
            .collect()
    }

    pub fn add_saved_view(&self, name: &str, query: &str) -> rusqlite::Result<i64> {
        self.conn.execute("INSERT INTO saved_view (name, query) VALUES (?1, ?2)", [name, query])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_saved_view(&self, view: &SavedView) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE saved_view SET name = ?2, query = ?3, icon = ?4 WHERE id = ?1",
            params![view.id, view.name, view.query, view.icon],
        )?;
        Ok(())
    }

    pub fn delete_saved_view(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM saved_view WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn insert_review(tx: &Transaction, card_id: &str, r: &LoggedReview) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO review_log
           (card_id, reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            card_id,
            r.reviewed_at,
            r.rating as i64,
            r.elapsed_days,
            r.memory.stability,
            r.memory.difficulty,
            r.interval_secs,
            r.was_new,
        ],
    )?;
    Ok(())
}

fn upsert_state(tx: &Transaction, card_id: &str, state: &CardState) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO card_state (card_id, stability, difficulty, due, last_review, reps, lapses)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (card_id) DO UPDATE SET
           stability = excluded.stability, difficulty = excluded.difficulty,
           due = excluded.due, last_review = excluded.last_review,
           reps = excluded.reps, lapses = excluded.lapses",
        params![
            card_id,
            state.memory.stability,
            state.memory.difficulty,
            state.due,
            state.last_review,
            state.reps,
            state.lapses,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_views() {
        let store = Store::open_in_memory().unwrap();
        let leeches = SavedView { id: 1, name: "Leeches".into(), query: "is:leech".into(), icon: Some("🐞".into()) };
        assert_eq!(store.saved_views().unwrap(), [leeches.clone()]);

        let id = store.add_saved_view("Due soon", "due<3").unwrap();
        let mut due = SavedView { id, name: "Due soon".into(), query: "due<3".into(), icon: None };
        assert_eq!(store.saved_views().unwrap(), [leeches.clone(), due.clone()]);

        due.query = "due<7 -is:suspended".into();
        due.icon = Some("⏰".into());
        store.update_saved_view(&due).unwrap();
        store.delete_saved_view(leeches.id).unwrap();
        assert_eq!(store.saved_views().unwrap(), [due]);
    }

    #[test]
    fn tag_icons() {
        let store = Store::open_in_memory().unwrap();
        store.set_tag_icon("Korean", Some("🇰🇷")).unwrap();
        store.set_tag_icon("korean", Some("🐯")).unwrap();
        store.set_tag_icon("verbs", Some("🏃")).unwrap();
        store.set_tag_icon("VERBS", None).unwrap();
        assert_eq!(store.tag_icons().unwrap(), HashMap::from([("korean".into(), "🐯".into())]));
    }

    #[test]
    fn record_and_reload() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = Outcome {
            memory: Memory {
                stability: 2.0,
                difficulty: 5.0,
            },
            interval_secs: 86_400,
        };
        let s1 = store
            .record_review("abc", 1000, Rating::Good, 0.0, None, outcome)
            .unwrap();
        assert_eq!((s1.reps, s1.lapses, s1.due), (1, 0, 87_400));

        let s2 = store
            .record_review("abc", 90_000, Rating::Again, 1.0, Some(s1), outcome)
            .unwrap();
        assert_eq!((s2.reps, s2.lapses), (2, 1));
        assert_eq!(store.state("abc").unwrap(), Some(s2));
        assert_eq!(store.states().unwrap().len(), 1);
        assert_eq!(store.new_cards_since(0).unwrap(), 1);
        assert_eq!(store.reviews_since(0).unwrap(), 2);
        assert_eq!(store.reviews_since(5000).unwrap(), 1);
        let log = store.reviews("abc").unwrap();
        assert_eq!(log.iter().map(|r| r.rating).collect::<Vec<_>>(), [Rating::Good, Rating::Again]);
        assert!(log[0].was_new && !log[1].was_new);
        assert!(store.reviews("other").unwrap().is_empty());
    }
}
