import { getRequestEvent, isServer } from "solid-js/web";
import { createSignal } from "solid-js";

const COOKIE = "famulen_workgroup";

function readCookie(header: string | null | undefined): string {
  const pair = (header ?? "")
    .split(";")
    .map((p) => p.trim())
    .find((p) => p.startsWith(`${COOKIE}=`));
  return pair ? decodeURIComponent(pair.slice(COOKIE.length + 1)) : "all";
}

const [current, setCurrent] = createSignal(isServer ? "all" : readCookie(document.cookie));

/** The workgroup the console is narrowed to; it survives a reload. On the server it is read from each request. */
export function workgroup(): string {
  return isServer ? readCookie(getRequestEvent()?.request.headers.get("cookie")) : current();
}

export function chooseWorkgroup(id: string) {
  document.cookie = `${COOKIE}=${encodeURIComponent(id)}; path=/; max-age=31536000; samesite=lax`;
  setCurrent(id);
}

/** The colors a workgroup or a calendar can take. */
export const COLORS: [string, string][] = [
  ["青", "#2a78d6"],
  ["橙", "#E0622F"],
  ["青緑", "#0F6E68"],
  ["紫", "#6250d6"],
  ["緑", "#008300"],
];
