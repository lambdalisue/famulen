/**
 * Listening to the owner through the glasses' microphone: when they begin to
 * speak, when they have stopped, and what was heard so far.
 */

/** The glasses send 16 kHz, 16-bit signed little-endian, one channel. */
export const RATE = 16_000;

/** How long a quiet after speech ends what is said. */
export const SILENCE_MS = 1_500;
/** How long the owner may say nothing at all before listening gives up. */
export const NOTHING_MS = 8_000;
/** The longest one thing said may be. */
export const LONGEST_MS = 60_000;
/** How often what is said so far is heard again while the owner speaks. */
export const INTERIM_MS = 3_000;
/** How long into what is said it is heard again as it goes; past this, only when it ends. */
export const INTERIM_UNTIL_MS = 30_000;

/** Loudness above which a frame counts as speech, as a fraction of full scale. */
const SPEECH = 0.02;

/** How loud a frame of samples is, 0 to 1. */
export function loudness(pcm: Uint8Array): number {
  const view = new DataView(pcm.buffer, pcm.byteOffset, pcm.byteLength);
  const n = Math.floor(pcm.byteLength / 2);
  if (n === 0) return 0;
  let sum = 0;
  for (let i = 0; i < n; i++) {
    const s = view.getInt16(i * 2, true) / 32768;
    sum += s * s;
  }
  return Math.sqrt(sum / n);
}

export type Heard = "go on" | "done" | "nothing";

/**
 * What has been heard of one thing said. Time is counted in the samples
 * themselves, so it runs as the sound arrives and not by the clock.
 */
export class Listening {
  #chunks: Uint8Array[] = [];
  #bytes = 0;
  #spokeAt: number | undefined;
  #lastSpeech = 0;
  #heardAt = 0;

  /** Take a frame and say whether to go on listening. */
  take(pcm: Uint8Array): Heard {
    this.#chunks.push(pcm);
    this.#bytes += pcm.byteLength;
    const now = this.ms();
    if (loudness(pcm) >= SPEECH) {
      this.#spokeAt ??= now;
      this.#lastSpeech = now;
    }
    if (now >= LONGEST_MS) return this.#spokeAt === undefined ? "nothing" : "done";
    if (this.#spokeAt === undefined) return now >= NOTHING_MS ? "nothing" : "go on";
    return now - this.#lastSpeech >= SILENCE_MS ? "done" : "go on";
  }

  /** How much has been heard, in milliseconds. */
  ms(): number {
    return (this.#bytes / 2 / RATE) * 1000;
  }

  spoke(): boolean {
    return this.#spokeAt !== undefined;
  }

  /** Whether it is time to hear again what has been said so far. */
  due(): boolean {
    if (!this.spoke() || this.ms() > INTERIM_UNTIL_MS || this.ms() - this.#heardAt < INTERIM_MS) return false;
    this.#heardAt = this.ms();
    return true;
  }

  /** Everything heard, as one run of samples. */
  sound(): Uint8Array {
    const out = new Uint8Array(this.#bytes);
    let at = 0;
    for (const c of this.#chunks) {
      out.set(c, at);
      at += c.byteLength;
    }
    return out;
  }
}
