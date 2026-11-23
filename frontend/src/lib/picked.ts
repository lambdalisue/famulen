import { createAsync } from "@solidjs/router";
import { createEffect } from "solid-js";
import { useParam } from "./params";
import { recordQuery } from "./records";
import { setView } from "./view";

/**
 * The record a page's address picks, and the page named by it, so what the
 * owner says to the secretary carries what is on screen.
 */
export function usePicked(of: { kind: () => string; section: () => string; sub?: () => string }) {
  const id = useParam("id");
  // A record that cannot be read is said where it is shown, not here.
  const record = createAsync(async () => (id() ? recordQuery(of.kind(), id()).catch(() => undefined) : undefined));
  createEffect(() => setView(of.section(), record()?.title ?? of.sub?.()));
  return { id, record };
}
