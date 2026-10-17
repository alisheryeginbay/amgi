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
}

export interface StudyState {
  card: StudyCard | null;
  revealed: boolean;
  autoplay: boolean;
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
  /** Missing without reviews to measure. */
  retention?: number | null;
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
  reps: number;
  lapses: number;
  /** Tagged `#suspended`, so never studied. */
  suspended: boolean;
}

export interface BrowseState {
  /** Changes when another deck, tag or saved view is browsed. */
  scope: string;
  /** The tag's or saved view's search, applied before the one typed. */
  query: string;
  cards: BrowseCard[];
}

export interface Environment {
  /** BCP 47, such as `en-GB`. */
  locale: string;
  /** 0 for Sunday through 6 for Saturday. */
  firstWeekday: number;
}

/** What the page sends back. */
export type Message =
  | { type: "ready" }
  | { type: "reveal" }
  | { type: "answer"; rating: 1 | 2 | 3 | 4 }
  | { type: "open"; path: string; line: number }
  | { type: "select"; ids: string[] }
  | { type: "contextCards"; ids: string[] }
  /** Cards matching the browse state's `query`, before the search typed. */
  | { type: "count"; count: number };
