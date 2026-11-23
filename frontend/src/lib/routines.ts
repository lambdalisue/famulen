import { query } from "@solidjs/router";
import { act } from "./act";
import { api } from "./api";

export const wakesQuery = query(async () => (await api.routines.listWakes({})).wakes, "wakes");

export function cancelWake(id: bigint) {
  return act(() => api.routines.cancelWake({ id }), "起床予約を取り消した");
}
