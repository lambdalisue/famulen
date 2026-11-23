import { NumberInput } from "@ark-ui/solid/number-input";
import { Show } from "solid-js";
import Segs from "~/components/Segs";
import type { LookoutDefault, LookoutSource } from "~/gen/famulen/v1/lookout_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createDraft } from "~/lib/draft";

/** The tool of a source that reads what arrived instead of calling a tool. */
export const ARRIVED = "arrived";

const FEEDS: Record<string, string> = { mail: "メール", chat: "チャット", other: "ほか" };

/** The default a source follows, as the owner reads it. */
const feedLabel = (feed: string) => FEEDS[feed] ?? feed;

/**
 * Whether a source follows the default for its feed or overrides it, with
 * the number asked only for an override. Each choice is saved as it is made.
 */
export default function LookoutInterval(props: {
  source: LookoutSource;
  defaults: LookoutDefault[];
  /** Where on the page it stands; one source can stand in several places. */
  place: "sources" | "own" | "connection";
  label?: string;
}) {
  const s = () => props.source;
  const label = () => props.label ?? `${s().name}を見る間隔（分）`;
  const inputId = () => `lookout-interval-${props.place}-${s().id}`;
  const [shown, setShown] = createDraft(() => String(s().intervalMinutes));
  const arrived = () => s().tool === ARRIVED;
  const fallback = () => {
    const minutes = props.defaults.find((d) => d.feed === s().feed)?.intervalMinutes;
    return minutes === undefined ? `既定（${feedLabel(s().feed)}）` : `既定（${feedLabel(s().feed)}・${minutes} 分）`;
  };
  const set = (intervalMinutes: number, done: string) =>
    act(async () => {
      await api.lookout.setLookoutInterval({ id: s().id, intervalMinutes });
      return true;
    }, done);
  const choose = async (way: string) => {
    if (way === "default" && s().ownInterval) {
      await set(0, "既定に戻した");
    } else if (way === "own" && !s().ownInterval) {
      const id = inputId();
      if (!(await set(s().intervalMinutes, "既定を上書きした"))) return;
      const input = document.getElementById(id);
      if (input instanceof HTMLInputElement) {
        input.focus();
        input.select();
      }
    }
  };
  return (
    <div class="lookout-interval-wrap">
      <Segs
        label={`${s().name}の間隔の決め方`}
        items={[
          ["default", fallback()],
          ["own", "上書き"],
        ]}
        value={s().ownInterval ? "own" : "default"}
        onChoose={(way) => void choose(way)}
      />
      <Show when={s().ownInterval}>
        <NumberInput.Root
          class="lookout-interval"
          ids={{ input: inputId() }}
          min={1}
          value={shown()}
          onValueChange={(d) => setShown(d.value)}
          onValueCommit={(d) => {
            const next = Math.trunc(d.valueAsNumber);
            if (!Number.isFinite(next) || next < 1) {
              setShown(String(s().intervalMinutes));
              return;
            }
            if (next !== s().intervalMinutes) void set(next, "間隔を変えた");
          }}
        >
          <NumberInput.Label class="visually-hidden">{label()}</NumberInput.Label>
          <NumberInput.Control>
            <NumberInput.Input class="input" inputmode="numeric" />
          </NumberInput.Control>
          <span class="sub">{arrived() ? "分ごとにまとめて見る" : "分ごとに読みに行く"}</span>
        </NumberInput.Root>
      </Show>
    </div>
  );
}
