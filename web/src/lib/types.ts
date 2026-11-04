// What the Swift side sends. Field names match `WebPayload.swift`.

export type CardKind = "basic" | "cloze";
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

export interface Environment {
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
