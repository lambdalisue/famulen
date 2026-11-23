import * as icons from "./icons";

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
  const detail = new URLSearchParams(search).has("id");
  if (path === "/settings/menu" || path === "/memory/menu") return { href: "/more", label: "ほか" };
  if (path.startsWith("/settings")) return { href: "/settings/menu", label: "設定" };
  if (path.startsWith("/memory")) return { href: "/memory/menu", label: "記憶" };
  for (const [prefix, label] of [["/tasks", "タスク"], ["/memos", "メモ"]]) {
    if (path === prefix && detail) {
      const routines = new URLSearchParams(search).get("tab") === "routines";
      return { href: routines ? `${prefix}?tab=routines` : prefix, label };
    }
  }
  for (const [prefix, label] of [["/parties", "相手"], ["/things", "物"], ["/contracts", "契約"]]) {
    if (path === prefix) return detail ? { href: prefix, label } : { href: "/more", label: "ほか" };
  }
  return undefined;
}
