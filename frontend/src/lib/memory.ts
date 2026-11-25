import { query } from "@solidjs/router";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { api } from "./api";
import { withParams } from "./params";

export const layersQuery = query(async () => (await api.memory.listLayers({})).layers, "memoryLayers");
export const layerQuery = query((layer: string, at: string) => api.memory.getLayer({ layer, at }), "memoryLayer");
export const noteOriginQuery = query((id: string) => api.memory.getNoteOrigin({ id }), "noteOrigin");
export const memoryQuery = query((id: string, path: string) => api.memory.getMemory({ id, path }), "memory");
export const memoryFileQuery = query((path: string) => api.memory.readFile({ path }), "memoryFile");
export const dreamsQuery = query(() => api.memory.listDreams({}), "dreams");
export const dreamSettingsQuery = query(() => api.memory.getDreamSettings({}), "dreamSettings");

/** Where each layer's page is; the dream is the section's front page. */
export function layerHref(id: string): string {
  return id === "dream" ? "/memory" : `/memory/${id}`;
}

/** The layer a memory file belongs to, by the first segment of its path. */
export function layerOfFile(path: string): string {
  const head = path.split("/")[0]?.replace(/\.md$/, "") ?? "";
  return ["core", "pending", "episodic", "working", "forgotten"].includes(head) ? head : "long";
}

/** Where a memory's details open: the page of its file, or over the list of its layer. */
export function itemHref(item: MemoryItem): string {
  const layer = layerOfFile(item.file);
  if (layer === "long" || layer === "working" || !item.id) return withParams("/memory/file", { path: item.file });
  const at = layer === "forgotten" ? item.file.replace(/^.*\/|\.md$/g, "") : "";
  return withParams(layerHref(layer), { at, item: item.id });
}

/** A commit's message without its trailers, and the sources one of those names. */
export function splitMessage(message: string): { text: string; sources: string[] } {
  const paragraphs = message.trim().split(/\n{2,}/);
  const last = paragraphs.length > 1 ? (paragraphs.at(-1)?.split("\n") ?? []) : [];
  if (last.length === 0 || !last.every((line) => /^[A-Za-z-]+: /.test(line))) return { text: message.trim(), sources: [] };
  const sources = last.flatMap((line) => /^Notebook-Source: (.+)$/.exec(line)?.[1].split(/,\s*/) ?? []);
  return { text: paragraphs.slice(0, -1).join("\n\n"), sources };
}

/** What a memory's prose says of where it came from, apart from the rest of it. */
export function splitSources(prose: string): { prose: string; sources: string[] } {
  const sources: string[] = [];
  let fence: string | undefined;
  const lines = prose.split("\n").filter((line) => {
    const mark = /^\s*(`{3,}|~{3,})/.exec(line)?.[1];
    if (fence !== undefined) {
      if (mark?.startsWith(fence)) fence = undefined;
      return true;
    }
    fence = mark;
    const said = /^(?:出どころ|出所)[：:]\s*(.+)$/.exec(line)?.[1];
    if (!said) return true;
    sources.push(...said.split(/[、,]\s*/).map((s) => s.trim()).filter(Boolean));
    return false;
  });
  return { prose: lines.join("\n").trim(), sources };
}

/**
 * Whether an item shows while `workgroup` is chosen: one marked for other
 * workgroups is left out; an unmarked one belongs to every workgroup.
 */
export function visible(item: MemoryItem, workgroup: string): boolean {
  const marked = (item.attrs.workgroup ?? "").split(",").filter((s) => s !== "");
  return workgroup === "all" || marked.length === 0 || marked.includes(workgroup);
}

/** Where a wiki link to a memory file opens. */
export function memoryHref(target: string): string | undefined {
  const m = /^(?:\.?\/)?memory\/(.+?)(?:\.md)?(?:#.*)?$/.exec(target);
  return m ? `/memory/file?path=${encodeURIComponent(`${m[1]}.md`)}` : undefined;
}
