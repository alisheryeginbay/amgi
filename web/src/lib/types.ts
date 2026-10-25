// What the Swift side sends. Field names match `WebPayload.swift`.

export type CardKind = "basic" | "cloze";
export type Pane = "study" | "dashboard" | "browse";

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
}

export interface DashboardState {
  due: number;
  newToday: number;
  reviewedToday: number;
  streak: number;
  /** Reviews per study day, oldest first, ending today. */
  activity: number[];
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

export interface BrowseCard {
  id: string;
  deck: string;
  tags: string[];
  path: string;
  line: number;
  kind: CardKind;
  front: string;
  back: string;
  /** Unix seconds; null for a new card. */
  due: number | null;
  /** Seconds from the last review to the due date; null for a new card. */
  interval: number | null;
  reps: number;
  lapses: number;
  /** Tagged `#suspended`, so never studied. */
  suspended: boolean;
  /** The note type of the note the card comes from; null for `Q:` and `C:` cards. */
  noteType: string | null;
  /** The note's fields as Markdown, in its note type's order; empty without a note. */
  fields: string[];
}

export interface NoteTypeInfo {
  name: string;
  fields: string[];
}

/** Which of the cards to browse. */
export interface BrowseState {
  /** Changes when another deck, tag or saved view is browsed. */
  scope: string;
  /** Only this deck and its subdecks; left out for every deck. */
  deck?: string;
  /** The tag's or saved view's search, applied before the one typed. */
  query: string;
}

export interface Environment {
  /** BCP 47, such as `en-GB`. */
  locale: string;
  /** 0 for Sunday through 6 for Saturday. */
  firstWeekday: number;
  /** The Mac's first CJK language, which picks how Han characters are drawn. */
  han?: "ko" | "ja" | "zh-hans" | "zh-hant";
  /** When the current study day started, in Unix seconds; absent with no collection open. */
  dayStart?: number;
  /** The keys that rate Again, Hard, Good and Easy, lowercase, from the config file. */
  answerKeys: string[];
}

/** What showed an answer or rated a card. */
export type Input = "key" | "pointer";

/** What the page sends back. */
export type Message =
  | { type: "ready" }
  | { type: "reveal"; via: Input }
  | { type: "answer"; rating: 1 | 2 | 3 | 4; via: Input }
  | { type: "open"; path: string; line: number }
  | { type: "select"; ids: string[] }
  | { type: "contextCards"; ids: string[] }
  /** Show a card in the Card Info window. */
  | { type: "info"; id: string }
  /** Cards matching the browse state's `query`, before the search typed. */
  | { type: "count"; count: number };
