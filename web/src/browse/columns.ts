import type { BrowseCard } from "../lib/types";

export type ColumnId = "question" | "answer" | "deck" | "tags" | "due" | "reviews" | "lapses";

export interface Column {
  id: ColumnId;
  title: string;
  width: number;
  minWidth: number;
  numeric?: boolean;
  sortable: boolean;
}

export const COLUMNS: Column[] = [
  { id: "question", title: "Question", width: 260, minWidth: 80, sortable: true },
  { id: "answer", title: "Answer", width: 220, minWidth: 80, sortable: true },
  { id: "deck", title: "Deck", width: 100, minWidth: 50, sortable: true },
  { id: "tags", title: "Tags", width: 110, minWidth: 40, sortable: false },
  { id: "due", title: "Due", width: 90, minWidth: 60, sortable: true },
  { id: "reviews", title: "Reviews", width: 56, minWidth: 40, numeric: true, sortable: true },
  { id: "lapses", title: "Lapses", width: 52, minWidth: 40, numeric: true, sortable: true },
];

export interface Sort {
  column: ColumnId;
  ascending: boolean;
}

/** Cards sorted by `sort`; ties keep file order. */
export function sorted(cards: BrowseCard[], sort: Sort | null, collator: Intl.Collator): BrowseCard[] {
  if (!sort) return cards;
  const compare = (a: BrowseCard, b: BrowseCard): number => {
    switch (sort.column) {
      case "question": return collator.compare(a.front, b.front);
      case "answer": return collator.compare(a.back, b.back);
      case "deck": return collator.compare(a.deck, b.deck);
      // New cards sort after every due date.
      case "due": return (a.due ?? Infinity) - (b.due ?? Infinity);
      case "reviews": return a.reps - b.reps;
      case "lapses": return a.lapses - b.lapses;
      case "tags": return 0;
    }
  };
  const direction = sort.ascending ? 1 : -1;
  return cards
    .map((card, index) => ({ card, index }))
    .sort((a, b) => direction * compare(a.card, b.card) || a.index - b.index)
    .map(({ card }) => card);
}

/** Last path component of a deck name: `science/physics` → `physics`. */
export function deckTitle(name: string): string {
  return name.split("/").pop() || name;
}
