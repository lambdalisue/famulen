/*
 * The service worker: it shows the notices Famulen sends and opens the
 * request one is about. It keeps no pages, so the console is always the
 * newest.
 */

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("push", (event) => {
  let notice = {};
  try {
    notice = event.data?.json() ?? {};
  } catch {
    // Something is shown all the same: Safari takes a push that shows
    // nothing as a reason to stop taking any.
  }
  event.waitUntil(
    self.registration.showNotification(notice.title ?? "Famulen", {
      body: notice.body ?? "",
      icon: notice.icon,
      tag: notice.tag,
      lang: "ja",
      data: { confirm: notice.confirm },
    }),
  );
});

/** Whether a window took a request; one still loading, or showing something else, does not answer. */
function told(window, confirm) {
  return new Promise((done) => {
    const { port1, port2 } = new MessageChannel();
    port1.onmessage = () => done(true);
    setTimeout(() => done(false), 3000);
    window.postMessage({ confirm }, [port2]);
  });
}

async function open(confirm) {
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  const shown = windows.find((w) => w.focused) ?? windows.find((w) => w.visibilityState === "visible") ?? windows[0];
  if (!shown) {
    await self.clients.openWindow(confirm ? `/?confirm=${encodeURIComponent(confirm)}` : "/");
    return;
  }
  // Told rather than led there, so that what is being written stays.
  const took = confirm ? told(shown, String(confirm)) : true;
  // A browser may refuse the focus; the request is shown there all the same.
  await shown.focus().catch(() => {});
  if (!(await took)) await shown.navigate(`/?confirm=${encodeURIComponent(confirm)}`);
}

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(open(event.notification.data?.confirm));
});
