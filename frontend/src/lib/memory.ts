import { query } from "@solidjs/router";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { api } from "./api";

export const layersQuery = query(async () => (await api.memory.listLayers({})).layers, "memoryLayers");
export const layerQuery = query((layer: string, at: string) => api.memory.getLayer({ layer, at }), "memoryLayer");
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

/**
 * Whether an item shows while `workgroup` is chosen: one marked for other
 * workgroups is left out; an unmarked one belongs to every workgroup.
 */
export function visible(item: MemoryItem, workgroup: string): boolean {
  const marked = (item.attrs.workgroup ?? "").split(",").filter((s) => s !== "");
  return workgroup === "all" || marked.length === 0 || marked.includes(workgroup);
}

/** Whether an episodic note is still to be read by a dream. */
export function waitsForDream(item: MemoryItem): boolean {
  return item.attrs.state === "waiting" && item.attrs.dreamt === undefined;
}

/** Where a wiki link to a memory file opens. */
export function memoryHref(target: string): string | undefined {
  const m = /^(?:\.?\/)?memory\/(.+?)(?:\.md)?(?:#.*)?$/.exec(target);
  return m ? `/memory/file?path=${encodeURIComponent(`${m[1]}.md`)}` : undefined;
}
