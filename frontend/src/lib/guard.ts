/**
 * Which hosts the console answers to: this machine, the console URL the
 * owner opens, and any names in FAMULEN_CONSOLE_HOSTS. The console has no
 * login, so a page on another site must not reach it through a name that
 * happens to resolve here.
 */
export function allowedHosts(env: Record<string, string | undefined>): string[] {
  const names = ["localhost", "127.0.0.1", "[::1]"];
  const url = env.FAMULEN_CONSOLE_URL;
  if (url) {
    try {
      names.push(new URL(url).hostname.toLowerCase());
    } catch {
      // A malformed URL adds nothing; the backend reports it.
    }
  }
  for (const h of (env.FAMULEN_CONSOLE_HOSTS ?? "").split(",")) {
    if (h.trim()) names.push(h.trim().toLowerCase());
  }
  return names;
}

function hostname(host: string): string {
  const h = host.toLowerCase();
  if (h.startsWith("[")) return h.slice(0, h.indexOf("]") + 1);
  return h.split(":")[0];
}

/** Where the notebook's Git is, which copies on the owner's other machines reach. */
export const NOTEBOOK_GIT = "/notebook.git/";

/**
 * Whether Git itself sends a request. It names no site, as a browser always
 * does, and its content type is one a page cannot send to another site
 * without asking first, which nothing here answers.
 */
function fromGit(request: Request): boolean {
  return (
    request.headers.get("origin") === null &&
    new URL(request.url).pathname.startsWith(NOTEBOOK_GIT) &&
    /^application\/x-git-(upload|receive)-pack-request$/.test(request.headers.get("content-type") ?? "")
  );
}

/** Why a request is refused, or undefined when it may go on. */
export function refuse(request: Request, hosts: string[]): { status: number; text: string } | undefined {
  const host = request.headers.get("host") ?? "";
  if (!hosts.includes(hostname(host))) return { status: 421, text: "unknown host" };
  const changes = request.method !== "GET" && request.method !== "HEAD";
  if (changes && !fromGit(request)) {
    const origin = request.headers.get("origin") ?? "";
    const authority = origin.replace(/^https?:\/\//, "");
    if (authority.toLowerCase() !== host.toLowerCase()) {
      return { status: 403, text: "changes must come from this site's own pages" };
    }
  }
  return undefined;
}
