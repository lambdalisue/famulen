import { createSignal, onCleanup } from "solid-js";
import { isServer } from "solid-js/web";
import { keep, kept } from "./storage";

export type Theme = "system" | "light" | "dark";

const KEY = "famulen-theme";

/** The ground at the very top of a page, which a phone's own bar takes the color of. */
export const GROUND = { light: "#FFFFFF", dark: "#171A24" } as const;

/**
 * Run before anything is drawn, so that the chosen theme never flashes past
 * as the system's. What the system chooses needs no script.
 */
export const EARLY = `try{const t=localStorage.getItem("${KEY}"),g=${JSON.stringify(GROUND)};if(Object.hasOwn(g,t)){document.documentElement.dataset.scheme=t;for(const m of document.querySelectorAll("meta[name=theme-color]"))m.content=g[t]}}catch{}`;

const [theme, setTheme] = createSignal<Theme>("system");
const [systemDark, setSystemDark] = createSignal(false);

/** What the owner chose on this browser. */
export { theme };

/** Whether the page is drawn dark, as chosen or as the system has it. */
export const dark = () => theme() === "dark" || (theme() === "system" && systemDark());

function read(): Theme {
  const t = kept(KEY);
  return t === "light" || t === "dark" ? t : "system";
}

function show(t: Theme) {
  setTheme(t);
  if (t === "system") delete document.documentElement.dataset.scheme;
  else document.documentElement.dataset.scheme = t;
  for (const meta of document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]')) {
    meta.content = GROUND[t === "system" ? (meta.media.includes("dark") ? "dark" : "light") : t];
  }
}

export function chooseTheme(t: Theme) {
  keep(KEY, t === "system" ? undefined : t);
  show(t);
}

/** Keep to the theme: as it was chosen before, as another tab chooses it, as the system turns. */
export function followTheme() {
  if (isServer) return;
  const system = window.matchMedia("(prefers-color-scheme: dark)");
  const turned = () => setSystemDark(system.matches);
  const stored = (e: StorageEvent) => {
    // Clearing all of the storage names no key.
    if (e.key === KEY || e.key === null) show(read());
  };
  // A page brought back from where the browser kept it was told nothing meanwhile.
  const back = (e: PageTransitionEvent) => {
    if (e.persisted) show(read());
  };
  turned();
  show(read());
  system.addEventListener("change", turned);
  window.addEventListener("storage", stored);
  window.addEventListener("pageshow", back);
  onCleanup(() => {
    system.removeEventListener("change", turned);
    window.removeEventListener("storage", stored);
    window.removeEventListener("pageshow", back);
  });
}
