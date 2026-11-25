import { createSignal } from "solid-js";
import { message } from "~/lib/act";

/** How many files one message may carry, as the backend takes them. */
const MOST = 10;
const LARGEST = 20 * 1024 * 1024;

/**
 * What the file picker offers. Naming JPEG and not HEIC has iOS hand its
 * photos over as JPEG.
 */
export const ACCEPTED =
  "image/png,image/jpeg,image/gif,image/webp,application/pdf,text/plain,text/markdown,text/csv,application/json,.md,.csv,.json,.log,.txt";

/** A file being attached to what is written. */
export interface Draft {
  name: string;
  /** Once it arrived. */
  id?: string;
  /** Why it did not arrive. */
  problem?: string;
}

export const attachmentUrl = (id: string) => `/attachments/${encodeURIComponent(id)}`;

/** A size as people say it. */
export function sizeOf(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

async function upload(file: File): Promise<string> {
  if (file.size > LARGEST) throw new Error("20 MB を超えている");
  const answer = await fetch(`/attachments?name=${encodeURIComponent(file.name)}`, {
    method: "POST",
    body: file,
    headers: { "content-type": "application/octet-stream" },
  });
  if (!answer.ok) throw new Error((await answer.text()) || `送れなかった（${answer.status}）`);
  return ((await answer.json()) as { id: string }).id;
}

/** The files attached to what is being written, sent on as they are added. */
export function createDrafts() {
  const [drafts, setDrafts] = createSignal<Draft[]>([]);
  const settle = (draft: Draft, done: Partial<Draft>) =>
    setDrafts((all) => all.map((d) => (d === draft ? { ...d, ...done } : d)));
  const add = (files: Iterable<File>) => {
    for (const file of files) {
      const draft: Draft = { name: file.name || "名前なし" };
      if (drafts().length >= MOST) draft.problem = `一度に添えられるのは ${MOST} 個まで`;
      setDrafts((all) => [...all, draft]);
      if (draft.problem) continue;
      upload(file).then(
        (id) => settle(draft, { id }),
        (e) => settle(draft, { problem: message(e) }),
      );
    }
  };
  const remove = (gone: readonly Draft[]) => setDrafts((all) => all.filter((d) => !gone.includes(d)));
  return { drafts, add, remove };
}
