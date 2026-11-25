import * as icons from "./icons";
import { withParams } from "./params";

export type Tab = {
  key: string;
  label: string;
  path: string;
  /** Raw SVG from ./icons. */
  icon: string;
};

export const SECTIONS: Tab[] = [
  { key: "home", label: "ホーム", path: "/", icon: icons.home },
  { key: "calendar", label: "カレンダー", path: "/calendar", icon: icons.calendar },
  { key: "task", label: "タスク", path: "/tasks", icon: icons.listCheck },
  { key: "memo", label: "メモ", path: "/memos", icon: icons.notes },
  { key: "parties", label: "相手", path: "/parties", icon: icons.users },
  { key: "things", label: "物", path: "/things", icon: icons.deviceLaptop },
  { key: "contracts", label: "契約", path: "/contracts", icon: icons.fileCertificate },
];

export const ASIDE: Tab[] = [
  { key: "memory", label: "記憶", path: "/memory", icon: icons.brain },
  { key: "settings", label: "設定", path: "/settings", icon: icons.settings },
];

/** The tabs the narrow layout's bottom bar holds; the rest open from ほか. */
export const BOTTOM = ["home", "calendar", "task", "memo"];

/** The narrow layout's index of the tabs the bottom bar leaves out. */
const MORE: Tab = { key: "more", label: "ほか", path: "/more", icon: icons.layoutGrid };

export function tabOf(path: string): Tab {
  // A run is of a delegated task.
  if (path.startsWith("/runs/")) return tabOf("/tasks");
  const all = [...SECTIONS, ...ASIDE, MORE];
  return all.find((t) => t.path !== "/" && (path === t.path || path.startsWith(`${t.path}/`))) ?? SECTIONS[0];
}

/** The tab an `open` button of the secretary names. */
export function tabNamed(name: string): Tab | undefined {
  const alias: Record<string, string> = { memos: "memo", party: "parties" };
  const key = alias[name] ?? name;
  return [...SECTIONS, ...ASIDE].find((t) => t.key === key);
}

/**
 * Where the narrow layout's back button leads from `path`, and its label.
 * Pages under ほか lead back to their index, details to their list.
 */
export function wayBack(path: string, search: string): { href: string; label: string } | undefined {
  if (path === "/settings/menu" || path === "/memory/menu") return { href: "/more", label: "ほか" };
  if (path === "/settings/services" && search) return { href: "/settings/services", label: "サービス" };
  if (path.startsWith("/settings")) return { href: "/settings/menu", label: "設定" };
  if (path.startsWith("/memory")) return { href: "/memory/menu", label: "記憶" };
  if (path.startsWith("/runs/")) return { href: "/tasks?tab=delegated", label: "委託" };
  const params = new URLSearchParams(search);
  const picked = ["id", "event", "record"].filter((p) => params.has(p));
  if (picked.length > 0) return { href: withParams(`${path}${search}`, Object.fromEntries(picked.map((p) => [p, ""]))), label: tabOf(path).label };
  return BOTTOM.includes(tabOf(path).key) || path === MORE.path ? undefined : { href: MORE.path, label: MORE.label };
}
