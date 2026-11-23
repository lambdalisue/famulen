/**
 * What a service's own instructions say to paste into an AI app, read into
 * how to connect it: a URL, a command line, the settings JSON those apps
 * share, or a `claude mcp add` line.
 */
export type ServiceTarget = {
  how: "remote" | "api" | "local";
  name: string;
  /** The URL, or the command with its arguments. */
  target: string;
  openapi: string;
  env: [string, string][];
  header: string;
  key: string;
};

type Entry = { command?: unknown; args?: unknown; env?: unknown; url?: unknown; serverUrl?: unknown; headers?: unknown };

const PLACEHOLDER = /^<.*>$|^\$\{|^\{.*\}$|^\[.*\]$|\byour[ _-]|^x{3,}$|^\.{3}$|^…$/i;

/** A value to keep, or empty for one the instructions leave to the reader. */
const given = (value: string) => (PLACEHOLDER.test(value.trim()) ? "" : value);

const empty = (how: ServiceTarget["how"], name: string, target: string): ServiceTarget => ({
  how,
  name,
  target,
  openapi: "",
  env: [],
  header: "",
  key: "",
});

/** Split a command line into words, honouring quotes and line continuations. */
function words(line: string): string[] {
  const out: string[] = [];
  for (const m of line.replace(/\\\r?\n/g, " ").matchAll(/(?:"(?:[^"\\]|\\.)*"|'[^']*'|[^\s"'])+/g))
    out.push(
      m[0].replace(/"((?:[^"\\]|\\.)*)"|'([^']*)'/g, (_, d?: string, s?: string) =>
        d !== undefined ? d.replace(/\\(.)/g, "$1") : (s ?? ""),
      ),
    );
  return out;
}

const quoted = (word: string) => {
  if (word !== "" && !/[\s"']/.test(word)) return word;
  return word.includes('"') ? `'${word}'` : `"${word}"`;
};

function ofUrl(url: string, name = ""): ServiceTarget {
  const parsed = new URL(url);
  if (/(openapi|swagger)/i.test(parsed.pathname) || /\.(json|ya?ml)$/i.test(parsed.pathname))
    return { ...empty("api", name, parsed.origin), openapi: url };
  return empty("remote", name, url);
}

function withHeaders(target: ServiceTarget, headers: [string, string][]): ServiceTarget {
  const [first] = headers;
  if (!first) return target;
  const [header, value] = first;
  const key = given(value.replace(/^Bearer\s+/i, ""));
  return { ...target, header: /^authorization$/i.test(header) ? "" : header, key };
}

function ofCommand(command: string[], env: [string, string][], name = ""): ServiceTarget | undefined {
  const at = command.findIndex((w) => /(^|\/)mcp-remote(@|$)/.test(w));
  const url = at >= 0 ? command.slice(at + 1).find((w) => /^https?:\/\//.test(w)) : undefined;
  if (url) return ofUrl(url, name);
  if (command.length === 0) return undefined;
  return { ...empty("local", name, command.map(quoted).join(" ")), env: env.map(([k, v]) => [k, given(v)]) };
}

const strings = (value: unknown): [string, string][] =>
  value && typeof value === "object" ? Object.entries(value).map(([k, v]) => [k, String(v)]) : [];

function ofEntry(name: string, entry: Entry): ServiceTarget | undefined {
  const url = entry.url ?? entry.serverUrl;
  if (typeof url === "string" && URL.canParse(url))
    return withHeaders(empty("remote", name, url), strings(entry.headers));
  if (typeof entry.command !== "string") return undefined;
  const args = Array.isArray(entry.args) ? entry.args.map(String) : [];
  const command = args.length > 0 ? [entry.command, ...args] : words(entry.command);
  return ofCommand(command, strings(entry.env), name);
}

function ofJson(text: string): ServiceTarget[] {
  let doc: unknown;
  for (const candidate of [text, `{${text.replace(/,\s*$/, "")}}`]) {
    try {
      doc = JSON.parse(candidate);
      break;
    } catch {
      // Not this form.
    }
  }
  if (!doc || typeof doc !== "object") return [];
  const top = doc as Record<string, unknown> & { mcp?: { servers?: unknown } };
  const servers = top.mcpServers ?? top.servers ?? top.mcp?.servers;
  const entries: [string, unknown][] =
    servers && typeof servers === "object"
      ? Object.entries(servers)
      : "command" in top || "url" in top || "serverUrl" in top
        ? [["", top]]
        : Object.entries(top);
  return entries.flatMap(([name, entry]) => {
    const found = entry && typeof entry === "object" ? ofEntry(name, entry as Entry) : undefined;
    return found ? [found] : [];
  });
}

/** `claude mcp add [options] <name> <url, or -- command>`, and `add-json`. */
function ofClaude(line: string[]): ServiceTarget[] {
  if (line[2] === "add-json") {
    const rest = line.slice(3).filter((w) => !w.startsWith("-"));
    const json = rest.find((w) => w.startsWith("{"));
    const name = rest.find((w) => !w.startsWith("{")) ?? "";
    return json ? ofJson(`{${JSON.stringify(name)}:${json}}`) : [];
  }
  const env: [string, string][] = [];
  const headers: [string, string][] = [];
  const plain: string[] = [];
  let i = 3;
  for (; i < line.length; i++) {
    const w = line[i]!;
    if (w === "--") {
      plain.push(...line.slice(i + 1));
      break;
    }
    const flag = /^(--?[a-z-]+)(?:=(.*))?$/.exec(w);
    if (!flag) {
      plain.push(w);
      continue;
    }
    const value = flag[2] ?? line[++i] ?? "";
    if (flag[1] === "-e" || flag[1] === "--env") {
      const cut = value.indexOf("=");
      if (cut > 0) env.push([value.slice(0, cut), value.slice(cut + 1)]);
    } else if (flag[1] === "-H" || flag[1] === "--header") {
      const cut = value.indexOf(":");
      if (cut > 0) headers.push([value.slice(0, cut).trim(), value.slice(cut + 1).trim()]);
    }
  }
  const [name = "", ...target] = plain;
  const first = target[0] ?? "";
  if (URL.canParse(first) && /^https?:/.test(first)) return [withHeaders(empty("remote", name, first), headers)];
  const found = ofCommand(target, env, name);
  return found ? [found] : [];
}

/** Every way to connect `text` describes; none when it says nothing usable. */
export function serviceTargets(text: string): ServiceTarget[] {
  const pasted = text.trim().replace(/^```\w*\s*|\s*```$/g, "").replace(/^\$\s+/, "");
  if (!pasted) return [];
  if (/^\{|^"[^"]+"\s*:/.test(pasted)) return ofJson(pasted);
  if (/^https?:\/\/\S+$/.test(pasted)) return URL.canParse(pasted) ? [ofUrl(pasted)] : [];
  if (/^https?:/.test(pasted)) return [];
  if (/^(localhost|127\.0\.0\.1)(:\d+)?(\/\S*)?$/.test(pasted)) return [ofUrl(`http://${pasted}`)];
  if (/^[\w-]+(\.[\w-]+)+(:\d+)?\/\S*$/.test(pasted) || /^(mcp|api)\.[\w.-]+$/.test(pasted))
    return [ofUrl(`https://${pasted}`)];
  const line = words(pasted);
  if (line[0] === "claude" && line[1] === "mcp") return ofClaude(line);
  const env: [string, string][] = [];
  while (line[0] && /^[A-Za-z_]\w*=/.test(line[0])) {
    const [head = ""] = line.splice(0, 1);
    const cut = head.indexOf("=");
    env.push([head.slice(0, cut), head.slice(cut + 1)]);
  }
  const found = ofCommand(line, env);
  return found ? [found] : [];
}

/** Whether a search looks like something to connect rather than a name. */
export function looksLikeTarget(text: string): boolean {
  const t = text.trim();
  return /^\{|:\/\/|^"[^"]+"\s*:|^[\w-]+(\.[\w-]+)+\/|^(npx|uvx|docker|node|python3?|claude|bunx|pnpm)\s/.test(t);
}
