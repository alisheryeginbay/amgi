import type { BrowseState, DashboardState, Environment, Message, Pane, StudyState } from "./types";

/** Everything the page shows, as last sent by the app. Each part is replaced whole,
 *  never changed in place, so none of it is deeply reactive. */
class AppState {
  pane = $state.raw<Pane | null>(null);
  study = $state.raw<StudyState | null>(null);
  dashboard = $state.raw<DashboardState | null>(null);
  browse = $state.raw<BrowseState | null>(null);
  search = $state.raw("");
  /** Lowercased tag → icon, such as an emoji. */
  icons = $state.raw<Record<string, string>>({});
  environment = $state.raw<Environment>({ locale: navigator.language, firstWeekday: 1 });
}

export const app = new AppState();

type Part = "pane" | "study" | "dashboard" | "browse" | "search" | "icons" | "environment";

/** Commands from the app, such as `replay`. */
export const commands = new EventTarget();

declare global {
  interface Window {
    amgi: { receive(part: Part, json: string): void; command(name: string): void; key(key: string): void };
    webkit?: { messageHandlers: { amgi: { postMessage(message: Message): void } } };
  }
}

window.amgi = {
  receive(part, json) {
    (app[part] as unknown) = JSON.parse(json);
  },
  command(name) {
    commands.dispatchEvent(new Event(name));
  },
  // A study key pressed while focus is elsewhere in the window, such as the sidebar.
  key(key) {
    commands.dispatchEvent(new CustomEvent("key", { detail: key }));
  },
};

export function post(message: Message) {
  // Missing when the page is opened in a browser for development.
  window.webkit?.messageHandlers.amgi.postMessage(message);
}
