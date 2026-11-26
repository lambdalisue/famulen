import { useSearchParams } from "@solidjs/router";

/** The one value of a search parameter; empty when it is not given. */
export function oneParam(v: string | string[] | undefined): string {
  return (Array.isArray(v) ? v[0] : v) ?? "";
}

/** A search parameter of the page's address, as it is now. */
export function useParam(name: string): () => string {
  const [params] = useSearchParams();
  return () => oneParam(params[name]);
}

/** `path` with parameters set, or taken out where the value is empty. */
export function withParams(path: string, params: { [name: string]: string }): string {
  const [base, search = ""] = path.split("?");
  const all = new URLSearchParams(search);
  for (const [name, value] of Object.entries(params)) {
    if (value) all.set(name, value);
    else all.delete(name);
  }
  const text = all.toString();
  return text ? `${base}?${text}` : base;
}

/** Where `href` leads within the console, as the router names it, or undefined for another site. */
export function addressOf(href: string): string | undefined {
  const to = new URL(href, window.location.href);
  return to.origin === window.location.origin ? `${to.pathname}${to.search}${to.hash}` : undefined;
}
