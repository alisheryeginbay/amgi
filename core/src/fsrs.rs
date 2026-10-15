//! FSRS-6 memory model (https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm).
//!
//! There are no learning steps: a failed card comes back after [`AGAIN_DELAY_SECS`],
//! and reviews less than a day apart use the short-term stability formula.

/// FSRS-6 default parameters.
pub const DEFAULT_PARAMS: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835,
    0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542,
];

pub const AGAIN_DELAY_SECS: i64 = 10 * 60;
const DAY_SECS: f64 = 86_400.0;
const MIN_STABILITY: f64 = 0.001;
const MAX_INTERVAL_DAYS: f64 = 36_500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    pub const ALL: [Rating; 4] = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy];

    fn grade(self) -> f64 {
        self as i32 as f64
    }

    pub fn from_i64(v: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|r| *r as i64 == v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Memory {
    pub stability: f64,
    pub difficulty: f64,
}

/// Result of rating a card: its new memory state and when to show it next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub memory: Memory,
    pub interval_secs: i64,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    pub w: [f64; 21],
    pub desired_retention: f64,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler {
            w: DEFAULT_PARAMS,
            desired_retention: 0.9,
        }
    }
}

impl Scheduler {
    fn decay(&self) -> f64 {
        -self.w[20]
    }

    fn factor(&self) -> f64 {
        0.9f64.powf(1.0 / self.decay()) - 1.0
    }

    /// Probability of recall after `elapsed_days` with the given stability.
    pub fn retrievability(&self, stability: f64, elapsed_days: f64) -> f64 {
        (1.0 + self.factor() * elapsed_days.max(0.0) / stability).powf(self.decay())
    }

    /// Days until retrievability drops to the desired retention.
    pub fn interval_days(&self, stability: f64) -> f64 {
        stability / self.factor() * (self.desired_retention.powf(1.0 / self.decay()) - 1.0)
    }

    fn init_stability(&self, r: Rating) -> f64 {
        self.w[r as usize - 1].max(MIN_STABILITY)
    }

    fn init_difficulty(&self, grade: f64) -> f64 {
        self.w[4] - (self.w[5] * (grade - 1.0)).exp() + 1.0
    }

    fn next_difficulty(&self, d: f64, r: Rating) -> f64 {
        let delta = -self.w[6] * (r.grade() - 3.0);
        let damped = d + delta * (10.0 - d) / 9.0;
        let reverted = self.w[7] * self.init_difficulty(4.0) + (1.0 - self.w[7]) * damped;
        reverted.clamp(1.0, 10.0)
    }

    fn recall_stability(&self, m: Memory, retr: f64, r: Rating) -> f64 {
        let w = &self.w;
        let hard = if r == Rating::Hard { w[15] } else { 1.0 };
        let easy = if r == Rating::Easy { w[16] } else { 1.0 };
        m.stability
            * (w[8].exp()
                * (11.0 - m.difficulty)
                * m.stability.powf(-w[9])
                * ((w[10] * (1.0 - retr)).exp() - 1.0)
                * hard
                * easy
                + 1.0)
    }

    fn forget_stability(&self, m: Memory, retr: f64) -> f64 {
        let w = &self.w;
        let s = w[11]
            * m.difficulty.powf(-w[12])
            * ((m.stability + 1.0).powf(w[13]) - 1.0)
            * (w[14] * (1.0 - retr)).exp();
        s.min(m.stability / (w[17] * w[18]).exp())
    }

    fn short_term_stability(&self, s: f64, r: Rating) -> f64 {
        let w = &self.w;
        let mut inc = (w[17] * (r.grade() - 3.0 + w[18])).exp() * s.powf(-w[19]);
        if r >= Rating::Good {
            inc = inc.max(1.0);
        }
        s * inc
    }

    /// Memory state after rating a card. `memory` is `None` for a card never reviewed.
    pub fn next_memory(&self, memory: Option<Memory>, elapsed_days: f64, r: Rating) -> Memory {
        let Some(m) = memory else {
            return Memory {
                stability: self.init_stability(r),
                difficulty: self.init_difficulty(r.grade()).clamp(1.0, 10.0),
            };
        };
        let stability = if elapsed_days < 1.0 {
            self.short_term_stability(m.stability, r)
        } else {
            let retr = self.retrievability(m.stability, elapsed_days);
            if r == Rating::Again {
                self.forget_stability(m, retr)
            } else {
                self.recall_stability(m, retr, r)
            }
        };
        Memory {
            stability: stability.max(MIN_STABILITY),
            difficulty: self.next_difficulty(m.difficulty, r),
        }
    }

    /// Outcomes for all four ratings, indexed by `rating as usize - 1`.
    /// Intervals are whole days and kept strictly increasing from Hard to Easy.
    pub fn preview(&self, memory: Option<Memory>, elapsed_days: f64) -> [Outcome; 4] {
        let mut prev_days = 0.0;
        Rating::ALL.map(|r| {
            let memory = self.next_memory(memory, elapsed_days, r);
            let interval_secs = if r == Rating::Again {
                AGAIN_DELAY_SECS
            } else {
                let days = self
                    .interval_days(memory.stability)
                    .round()
                    .clamp(1.0, MAX_INTERVAL_DAYS)
                    .max(prev_days + 1.0);
                prev_days = days;
                (days * DAY_SECS) as i64
            };
            Outcome {
                memory,
                interval_secs,
            }
        })
    }

    pub fn schedule(&self, memory: Option<Memory>, elapsed_days: f64, r: Rating) -> Outcome {
        self.preview(memory, elapsed_days)[r as usize - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(o: &Outcome) -> f64 {
        o.interval_secs as f64 / DAY_SECS
    }

    #[test]
    fn interval_equals_stability_at_90_percent() {
        let s = Scheduler::default();
        assert!((s.interval_days(10.0) - 10.0).abs() < 1e-9);
        assert!((s.retrievability(10.0, 10.0) - 0.9).abs() < 1e-9);
    }

    #[test]
    fn new_card() {
        let s = Scheduler::default();
        let p = s.preview(None, 0.0);
        assert_eq!(p[0].interval_secs, AGAIN_DELAY_SECS);
        assert_eq!(days(&p[1]), 1.0);
        assert_eq!(days(&p[2]), 2.0);
        assert_eq!(days(&p[3]), 8.0);
        assert!((p[2].memory.stability - 2.3065).abs() < 1e-9);
        assert!(p[0].memory.difficulty > p[3].memory.difficulty);
    }

    #[test]
    fn successful_reviews_grow_intervals() {
        let s = Scheduler::default();
        let mut mem = None;
        let mut elapsed = 0.0;
        let mut last = 0.0;
        for _ in 0..6 {
            let o = s.schedule(mem, elapsed, Rating::Good);
            assert!(days(&o) > last, "{} <= {}", days(&o), last);
            last = days(&o);
            elapsed = last;
            mem = Some(o.memory);
        }
        assert!(last > 60.0);
    }

    #[test]
    fn lapse_shrinks_stability() {
        let s = Scheduler::default();
        let m = Memory {
            stability: 30.0,
            difficulty: 5.0,
        };
        let o = s.schedule(Some(m), 30.0, Rating::Again);
        assert!(o.memory.stability < 30.0);
        assert!(o.memory.difficulty > 5.0);
    }

    #[test]
    fn same_day_review_uses_short_term() {
        let s = Scheduler::default();
        let m = Memory {
            stability: 2.0,
            difficulty: 5.0,
        };
        let good = s.next_memory(Some(m), 0.01, Rating::Good);
        let again = s.next_memory(Some(m), 0.01, Rating::Again);
        assert!(good.stability >= 2.0);
        assert!(again.stability < 2.0);
    }
}
