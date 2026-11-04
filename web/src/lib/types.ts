// What the Swift side sends. Field names match `WebPayload.swift`.

export type CardKind = "basic" | "cloze";
export type Pane = "study" | "dashboard";
/** Which of today's counts a card to study comes from: never studied, answered Again
 *  and not passed since, or back after days. */
export type StudyStage = "new" | "learning" | "due";

export interface StudyCard {
  id: string;
  deck: string;
  tags: string[];
  /** Deck file, relative to the collection root. */
  path: string;
  line: number;
  kind: CardKind;
  /** Markdown. */
  front: string;
  /** Markdown. */
  back: string;
  stage: StudyStage;
  /** The answer to type before it shows, as Markdown; null unless the card asks for one. */
  typed?: string | null;
  /** The field that answer comes from, such as `Word`, which the typing box names. */
  typedField?: string | null;
}

export interface StudyState {
  card: StudyCard | null;
  revealed: boolean;
  autoplay: boolean;
  /** Whether a key made the last change, which then shows without animating. */
  instant: boolean;
  /** Whether the progress bar shows. */
  progress: boolean;
  /** Answers given today. */
  reviewed: number;
  /** Cards still to study today: due, learning and new. */
  remaining: number;
  /** The last answer made its card a leech; `id` is new for each one. */
  leech: LeechNotice | null;
}

export interface LeechNotice {
  id: number;
  /** Times the card has been forgotten. */
  lapses: number;
  /** Whether it was suspended for it. */
  suspended: boolean;
}

export interface DashboardState {
  due: number;
  newToday: number;
  reviewedToday: number;
  /** Seconds spent studying today, counting timed reviews. */
  studySecsToday: number;
  /** About how long the cards left today will take; null without timed reviews to go by. */
  secsLeft: number | null;
  streak: number;
  /** Reviews per study day, oldest first, ending today. */
  activity: number[];
  /** Seconds spent on each day of `activity`, counting timed reviews. */
  activitySecs: number[];
  /** Average seconds per review over the last 30 days; null without timed reviews. */
  secsPerReview: number | null;
  /** Average seconds a card studied before takes the day it comes back, its repeats
   *  that day included, over the last 30 days; estimates the forecast's time. */
  secsPerDueCard: number | null;
  /** Unix seconds at which the first day of `activity` starts. */
  activityStart: number;
  /** Cards due per day from today on; today includes overdue cards. */
  forecast: number[];
  newCards: number;
  youngCards: number;
  matureCards: number;
  /** Tagged `#suspended`; not counted as new, young or mature. */
  suspendedCards: number;
  /** Today, yesterday, and the last 7, 30 and 365 days. */
  retention: Retention[];
}

/** Reviews of cards last seen a day or more before; failed is rated Again. */
export interface Retention {
  youngPassed: number;
  youngFailed: number;
  maturePassed: number;
  matureFailed: number;
}

export interface Environment {
  /** BCP 47, such as `en-GB`. */
  locale: string;
  /** 0 for Sunday through 6 for Saturday. */
  firstWeekday: number;
  /** The Mac's first CJK language, which picks how Han characters are drawn. */
  han?: "ko" | "ja" | "zh-hans" | "zh-hant";
  /** The keys that rate Again, Hard, Good and Easy, lowercase, from the config file. */
  answerKeys: string[];
}

/** What showed an answer or rated a card. */
export type Input = "key" | "pointer";

/** What the page sends back. */
export type Message =
  | { type: "ready" }
  | { type: "reveal"; via: Input }
  | { type: "answer"; rating: 1 | 2 | 3 | 4; via: Input };
