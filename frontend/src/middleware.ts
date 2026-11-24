import { createMiddleware } from "@solidjs/start/middleware";
import { backendUrl } from "./lib/api";
import { allowedHosts, refuse } from "./lib/guard";

/** Paths the browser calls that belong to the backend. */
function relayed(path: string): boolean {
  return path.startsWith("/famulen.v1.") || path === "/health";
}

const hosts = allowedHosts(process.env);

export default createMiddleware([
  async (event, next) => {
    const refused = refuse(event.req, hosts);
    if (refused) return new Response(refused.text, { status: refused.status });
    const url = new URL(event.req.url);
    if (!relayed(url.pathname)) return next();
    const headers = new Headers(event.req.headers);
    headers.delete("host");
    // Checked here; the backend would take the console's site for another.
    headers.delete("origin");
    // Node's fetch sends a streamed body only with `duplex: "half"`, which
    // the DOM's RequestInit does not name yet.
    const init: RequestInit & { duplex: "half" } = {
      method: event.req.method,
      headers,
      body: event.req.method === "GET" || event.req.method === "HEAD" ? undefined : event.req.body,
      duplex: "half",
    };
    const answer = await fetch(`${backendUrl()}${url.pathname}${url.search}`, init).catch(() => undefined);
    if (!answer) return new Response("the backend does not answer", { status: 503 });
    // fetch has already decoded a compressed body, so passing its encoding
    // and length on would make the browser decode it again and fail.
    const answerHeaders = new Headers(answer.headers);
    answerHeaders.delete("content-encoding");
    answerHeaders.delete("content-length");
    return new Response(answer.body, { status: answer.status, headers: answerHeaders });
  },
]);
