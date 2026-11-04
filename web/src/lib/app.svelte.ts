import type { Environment, Message, StudyState } from "./types";

/** Everything the page shows, as last sent by the app. Each part is replaced whole,
 *  never changed in place, so none of it is deeply reactive. */
class AppState {
  study = $state.raw<StudyState | null>(null);
  environment = $state.raw<Environment>({ answerKeys: ["1", "2", "3", "4"] });
  /** Stylesheets of the CJK font packs downloaded so far. */
  fonts = $state.raw<string[]>([]);
}

export const app = new AppState();

type Part = "study" | "environment" | "fonts";

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
  // A study key pressed while focus is elsewhere in the window, such as a header button.
  key(key) {
    commands.dispatchEvent(new CustomEvent("key", { detail: key }));
  },
};

export function post(message: Message) {
  // Missing when the page is opened in a browser for development.
  window.webkit?.messageHandlers.amgi.postMessage(message);
}
