//! SQLite persistence. Card content lives in Markdown; the database only holds
//! scheduling state and the review log, keyed by content-derived card ids.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::fsrs::{Memory, Outcome, Rating};

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
        conn.pragma_update(None, "journal_mode", "WAL")?;
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
        tx.execute(
            "INSERT INTO review_log
               (card_id, reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                card_id,
                now,
                rating as i64,
                elapsed_days,
                state.memory.stability,
                state.memory.difficulty,
                outcome.interval_secs,
                prev.is_none(),
            ],
        )?;
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
        tx.commit()?;
        Ok(state)
    }

    /// Number of new cards first reviewed at or after `since`.
    pub fn new_cards_since(&self, since: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM review_log WHERE was_new AND reviewed_at >= ?1",
            [since],
            |r| r.get(0),
        )
    }

    pub fn reviews_since(&self, since: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM review_log WHERE reviewed_at >= ?1",
            [since],
            |r| r.get(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
