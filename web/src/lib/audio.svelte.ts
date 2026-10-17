import { SvelteSet } from "svelte/reactivity";
import { extension } from "./markdown";

const MIME: Record<string, string> = {
  mp3: "audio/mpeg", m4a: "audio/mp4", m4b: "audio/mp4", aac: "audio/aac", wav: "audio/wav",
  aif: "audio/aiff", aiff: "audio/aiff", caf: "audio/x-caf", flac: "audio/flac",
  ogg: "audio/ogg", oga: "audio/ogg", opus: "audio/ogg; codecs=opus",
  mp4: "video/mp4", m4v: "video/mp4", mov: "video/quicktime", webm: "video/webm",
};

/** Plays card sounds one after another. Starting a new list stops the current one. */
class SoundPlayer {
  /** The sound playing or loading. */
  current = $state<string | null>(null);
  /** Sounds that are missing or could not be decoded. */
  readonly unplayable = new SvelteSet<string>();

  #audio = new Audio();
  #queue: string[] = [];

  constructor() {
    this.#audio.addEventListener("ended", () => this.#next());
    this.#audio.addEventListener("error", () => {
      // Only for the sound playing, not one stopped in favour of another.
      if (!this.current || this.#audio.getAttribute("src") !== this.current) return;
      this.unplayable.add(this.current);
      this.#next();
    });
  }

  /** False when the format is known not to play here, such as Ogg on some systems. */
  supports(src: string): boolean {
    const type = MIME[extension(src)];
    return !type || this.#audio.canPlayType(type) !== "";
  }

  play(sources: string[]) {
    this.stop();
    this.#queue = sources.filter((src) => !this.unplayable.has(src));
    this.#next();
  }

  stop() {
    this.#queue = [];
    this.current = null;
    this.#audio.pause();
    this.#audio.removeAttribute("src");
    this.#audio.load();
  }

  #next() {
    const src = this.#queue.shift() ?? null;
    this.current = src;
    if (!src) return;
    if (!this.supports(src)) {
      this.unplayable.add(src);
      this.#next();
      return;
    }
    this.#audio.src = src;
    // Failures arrive as `error` events; a rejection here is a newer `play` taking over.
    this.#audio.play().catch(() => {});
  }
}

export const player = new SoundPlayer();
