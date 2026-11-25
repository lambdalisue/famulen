import { api } from "./api";
import { keep, kept } from "./storage";

const WORKER = "/sw.js";
/** Kept while this browser is meant to take notices. */
const TAKING = "notices.taking";

/** Why this browser takes no notices, or undefined when it can. */
export type Lack = "insecure" | "home-screen" | "unsupported" | "denied";

/** iPads say they are Macs; only they have a touch screen. */
function isApplePhone(): boolean {
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
}

export function lack(): Lack | undefined {
  if (!window.isSecureContext) return "insecure";
  if (!("serviceWorker" in navigator && "PushManager" in window && "Notification" in window)) {
    const alone = window.matchMedia("(display-mode: standalone)").matches;
    return isApplePhone() && !alone ? "home-screen" : "unsupported";
  }
  if (Notification.permission === "denied") return "denied";
  return undefined;
}

/** Where this browser takes notices now. */
export async function subscription(): Promise<PushSubscription | undefined> {
  if (lack()) return undefined;
  const registration = await navigator.serviceWorker.getRegistration("/");
  return (await registration?.pushManager.getSubscription()) ?? undefined;
}

/** What the owner calls the browser they hold. */
export function deviceName(agent: string, alone: boolean, touch: boolean): string {
  const system = /iPhone/.test(agent)
    ? "iPhone"
    : /iPad/.test(agent) || (/Macintosh/.test(agent) && touch)
      ? "iPad"
      : /Android/.test(agent)
        ? "Android"
        : /Macintosh/.test(agent)
          ? "Mac"
          : /Windows/.test(agent)
            ? "Windows"
            : /Linux/.test(agent)
              ? "Linux"
              : "";
  const browser = /Edg\//.test(agent)
    ? "Edge"
    : /Firefox\//.test(agent)
      ? "Firefox"
      : /Chrome\//.test(agent)
        ? "Chrome"
        : /Safari\//.test(agent)
          ? "Safari"
          : "";
  return [system, alone ? "アプリ" : browser].filter(Boolean).join(" · ");
}

/** Wait until the worker runs; one that just registered has not started. */
async function worker(): Promise<ServiceWorkerRegistration> {
  await navigator.serviceWorker.register(WORKER, { scope: "/" });
  return navigator.serviceWorker.ready;
}

/**
 * Take notices in this browser. Call it from a press: browsers ask for the
 * permission only then.
 */
export async function takeNotices(key: string) {
  if ((await Notification.requestPermission()) !== "granted") throw new Error("通知が許可されなかった");
  const { pushManager } = await worker();
  // One Famulen does not know may be dead, or made for another key.
  await (await pushManager.getSubscription())?.unsubscribe();
  const made = await pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
  const { endpoint, keys } = made.toJSON();
  const alone = window.matchMedia("(display-mode: standalone)").matches;
  await api.notices.addNoticeDevice({
    endpoint: endpoint ?? "",
    p256dh: keys?.p256dh ?? "",
    auth: keys?.auth ?? "",
    name: deviceName(navigator.userAgent, alone, navigator.maxTouchPoints > 1),
  });
  keep(TAKING, "on");
}

export async function leaveNotices(id: bigint) {
  await api.notices.removeNoticeDevice({ id });
  keep(TAKING, undefined);
  await (await subscription())?.unsubscribe();
}

/** Follow the notices pressed while the console is open. */
export function followNotices(open: (confirm: string) => void): () => void {
  if (!("serviceWorker" in navigator)) return () => {};
  const told = (e: MessageEvent) => {
    const confirm: unknown = e.data?.confirm;
    if (typeof confirm !== "string") return;
    e.ports[0]?.postMessage("shown");
    open(confirm);
  };
  navigator.serviceWorker.addEventListener("message", told);
  navigator.serviceWorker.startMessages();
  return () => navigator.serviceWorker.removeEventListener("message", told);
}

/** Whether this browser was to take notices and takes them no more, as when its subscription ran out; told once. */
export async function lapsed(): Promise<boolean> {
  if (!kept(TAKING)) return false;
  const endpoint = (await subscription())?.endpoint ?? "";
  const { devices } = await api.notices.getNotices({ endpoint });
  if (endpoint && devices.some((d) => d.here)) return false;
  keep(TAKING, undefined);
  return true;
}
