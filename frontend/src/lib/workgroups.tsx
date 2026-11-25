import { createContext, type JSX, useContext } from "solid-js";
import type { Workgroup } from "~/gen/famulen/v1/workgroups_pb";
import { workgroupsQuery } from "./data";
import { createKept } from "./kept";

const Read = createContext<() => Workgroup[] | undefined>();

/**
 * Reads them once for all that is inside, for the reason `OwnerProvider` does:
 * a row drawn later would otherwise take its whole page off the screen.
 */
export function WorkgroupsProvider(props: { children?: JSX.Element }) {
  const workgroups = createKept(() => workgroupsQuery());
  return <Read.Provider value={() => workgroups.latest}>{props.children}</Read.Provider>;
}

/** Every workgroup there is. */
export function useWorkgroups() {
  const workgroups = useContext(Read);
  if (!workgroups) throw new Error("useWorkgroups needs a WorkgroupsProvider around it");
  return workgroups;
}
