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

/** `11月22日 04:00` in `tz`. */
export function nightLabel(at: bigint, tz: string): string {
  const parts = new Intl.DateTimeFormat("ja-JP", {
    timeZone: tz || "UTC",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(Number(at) * 1000));
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
  return `${get("month")}月${get("day")}日 ${get("hour")}:${get("minute")}`;
}

/** `2026-11-22 04:00` in `tz`. */
export function stamp(at: bigint, tz: string): string {
  return new Intl.DateTimeFormat("sv-SE", {
    timeZone: tz || "UTC",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(Number(at) * 1000));
}
