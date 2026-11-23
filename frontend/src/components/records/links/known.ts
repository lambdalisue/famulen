import { createContext, useContext } from "solid-js";
import type { RecordLink } from "~/gen/famulen/v1/records_pb";

/** What the records linked to on screen are called now, and which are gone. */
export const KnownLinks = createContext<() => RecordLink[]>(() => []);

export function useKnownLink() {
  const known = useContext(KnownLinks);
  return (kind: string, id: string) => known().find((l) => l.kind === kind && l.id === id);
}
