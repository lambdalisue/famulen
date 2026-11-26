import { createMiddleware } from "@solidjs/start/middleware";
import { backendUrl } from "./lib/api";
import { compressed } from "./lib/compress";
import { allowedHosts, NOTEBOOK_GIT, refuse } from "./lib/guard";

/** Paths the browser or Git calls that belong to the backend. */
function relayed(path: string): boolean {
  return path.startsWith("/famulen.v1.") || path.startsWith(NOTEBOOK_GIT) || /^\/(attachments|link-images|party-pictures|holding-pictures)(\/|$)/.test(path) || path === "/health";
}

/** What h3 makes of a rendered page before it becomes a response. */
type Rendered = { body: BodyInit | null; status?: number; statusText?: string; headers?: Headers };

const rendered = (v: unknown): v is Rendered => typeof v === "object" && v !== null && "body" in v;

const hosts = allowedHosts(process.env);

export default createMiddleware([
  async (event, next) => {
    const refused = refuse(event.req, hosts);
    if (refused) return new Response(refused.text, { status: refused.status });
    const url = new URL(event.req.url);
    const accept = event.req.headers.get("accept-encoding");
    if (!relayed(url.pathname)) {
      const page = await next();
      if (page instanceof Response) return compressed(page, accept);
      if (!rendered(page)) return page;
      // A rendered page comes as a body whose headers the event holds. They are
      // taken into the response here, as h3 would otherwise add them again over
      // it, or, for an error page, leave them out.
      const headers = new Headers(event.res.headers);
      page.headers?.forEach((value, name) => headers.set(name, value));
      for (const name of [...event.res.headers.keys()]) event.res.headers.delete(name);
      return compressed(new Response(page.body, { status: page.status ?? event.res.status, statusText: page.statusText, headers }), accept);
    }
    const headers = new Headers(event.req.headers);
    headers.delete("host");
    // These are about the connection they came on, and fetch refuses them.
    for (const name of ["connection", "expect", "keep-alive", "transfer-encoding", "upgrade"]) headers.delete(name);
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
    return compressed(new Response(answer.body, { status: answer.status, headers: answerHeaders }), accept);
  },
]);
