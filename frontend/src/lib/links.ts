import type { LinkPreview } from "~/gen/famulen/v1/console_pb";
import { api } from "./api";

/** What the links of a message lead to, once it was read; a message drawn anew shows it at once. */
const read = new Map<bigint, LinkPreview[]>();

/** How many messages are asked about at once: a slow page must not keep the browser's few connections from the rest. */
const AT_ONCE = 2;
let asking = 0;
const waiting: (() => void)[] = [];

const mayLink = /https?:\/\//;

export const hasLinks = (text: string) => mayLink.test(text);

export const previewsRead = (message: bigint) => read.get(message);

export async function readPreviews(message: bigint): Promise<LinkPreview[]> {
  // Who ends hands its turn over, so that no one comes in between.
  if (asking >= AT_ONCE) await new Promise<void>((turn) => waiting.push(turn));
  else asking += 1;
  try {
    const known = read.get(message);
    if (known) return known;
    const { previews } = await api.console.listLinkPreviews({ message });
    // A page that tells nothing now may tell of itself later.
    if (previews.length > 0) read.set(message, previews);
    return previews;
  } finally {
    const next = waiting.shift();
    if (next) next();
    else asking -= 1;
  }
}

export const linkImageUrl = (id: string) => `/link-images/${encodeURIComponent(id)}`;
