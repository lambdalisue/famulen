import { act } from "~/lib/act";
import { api } from "~/lib/api";
import type { Rec } from "~/lib/records";

type Save = Parameters<typeof api.records.saveRecord>[0];
export type Change = Omit<Save, "$typeName" | "kind" | "id">;

/** Saves a change, or one worked out from the record as it is by then. */
export type SaveRecord = (change: Change | ((now: Rec) => Change), done?: string) => Promise<Rec | undefined>;

/**
 * Save one change of a record after another, each worked out from the record
 * as it is by then: a section goes back whole, and what was saved a moment
 * ago, or by the secretary meanwhile, must not be lost with it.
 */
export function useRecordEdit(of: { kind: string; id: string }): SaveRecord {
  let last: Promise<unknown> = Promise.resolve();
  return (change, done = "保存した") => {
    const { kind, id } = of;
    const turn = last.then(() =>
      act(async () => {
        const what = typeof change === "function" ? change(await api.records.getRecord({ kind, id })) : change;
        return api.records.saveRecord({ kind, id, ...what });
      }, done),
    );
    last = turn;
    return turn;
  };
}
