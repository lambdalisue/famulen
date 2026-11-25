import { inboxQuery } from "./data";
import { createKept } from "./kept";

/** What waits for the owner, counted the same wherever it is shown. */
export function useWaiting() {
  const inbox = createKept(() => inboxQuery());
  const approvals = () => (inbox()?.approvals ?? []).filter((a) => a.status === "pending");
  const held = () => inbox()?.held ?? [];
  return { there: () => inbox() !== undefined, approvals, held, count: () => approvals().length + held().length };
}

/** The element of a request in the conversation. */
export const approvalId = (id: bigint | string) => `approval-${id}`;
