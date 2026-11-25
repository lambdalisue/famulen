import { createAsync, query } from "@solidjs/router";
import { createSignal, For, Match, Show, Switch } from "solid-js";
import { Button, Callout, Icon, Keyed, List, Section, Select, whenConfirmed } from "~/components/ui";
import { type PastPurchases as Mailbox, PastPurchasesState } from "~/gen/famulen/v1/lookout_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";

const pastQuery = query(async () => (await api.lookout.listPastPurchases({})).mailboxes, "pastPurchases");

/** How many years back, and what that is called. */
const SPANS: [string, string][] = [
  ["1", "1 年"],
  ["3", "3 年"],
  ["5", "5 年"],
  ["10", "10 年"],
  ["20", "20 年"],
];

/** `years` before `today`, both YYYY-MM-DD; a leap day falls to the first of March. */
function yearsBack(today: string, years: number): string {
  const d = new Date(`${today}T00:00:00Z`);
  d.setUTCFullYear(d.getUTCFullYear() - years);
  return d.toISOString().slice(0, 10);
}

function Row(props: { of: Mailbox }) {
  const owner = useOwner();
  const [years, setYears] = createSignal("3");
  const m = () => props.of;
  const going = () => m().state === PastPurchasesState.GOING;
  const start = (since: string, done: string) => void act(() => api.lookout.startPastPurchases({ connection: m().connection, since }), done);
  return (
    <div class="item past-purchases" role="listitem">
      <Icon svg={icons.deviceLaptop} size={16} />
      <div class="grow stack">
        <b>{m().name}</b>
        <span class="sub" role="status">
          <Switch fallback="まださかのぼっていない">
            <Match when={going() && m().searched}>探し終え、残りを読んでいる</Match>
            <Match when={going()}>{m().month.slice(0, 7)} を探している</Match>
            <Match when={m().state === PastPurchasesState.STOPPED}>{m().searched ? "読む途中" : m().month.slice(0, 7)} で止めている</Match>
            <Match when={m().state === PastPurchasesState.DONE}>{m().since} からのメールを探し終えた</Match>
          </Switch>
          <Show when={m().state !== PastPurchasesState.UNSPECIFIED}>
            {" "}
            · 見つけたメール {String(m().found)} 通 · 残り {String(m().left)} 通 · 足した物 {String(m().added)} 件
          </Show>
        </span>
        <Show when={m().problem}>
          <Callout tone="warn">{m().problem}</Callout>
        </Show>
      </div>
      <Show
        when={going()}
        fallback={
          <>
            <Show when={m().state === PastPurchasesState.STOPPED}>
              <Button small aria-label={`${m().name}を続ける`} onClick={() => start(m().since, "続きから始めた")}>
                続ける
              </Button>
            </Show>
            <Select aria-label={`${m().name}をさかのぼる年数`} value={years()} onChange={(e) => setYears(e.currentTarget.value)}>
              <For each={SPANS}>{([value, text]) => <option value={value}>{text}</option>}</For>
            </Select>
            <Button
              small
              aria-label={`${m().name}をさかのぼる`}
              onClick={() =>
                void whenConfirmed(
                  `${m().name}のメールを ${years()} 年さかのぼる`,
                  "さかのぼる",
                  async () => start(yearsBack(owner.today(), Number(years())), "さかのぼり始めた"),
                  "見つけたメールを 1 通ずつモデルに読ませ、買った物を確かめずに足す。見張りの 1 日の上限の半分まで使い、残りは次の日に続ける。",
                )
              }
            >
              さかのぼる
            </Button>
          </>
        }
      >
        <Button small aria-label={`${m().name}をさかのぼるのを止める`} onClick={() => void act(() => api.lookout.stopPastPurchases({ id: m().connection }), "止めた")}>
          止める
        </Button>
      </Show>
    </div>
  );
}

/** Going back over the mailboxes for what was bought before. */
export default function PastPurchases() {
  const mailboxes = createAsync(() => pastQuery());
  return (
    <Show when={(mailboxes() ?? []).length > 0}>
      <Section
        title="過去の買い物"
        note="注文や発送の知らせらしいメールを古い順に探し、買った物を「物」に足す。新しく届くメールは、ここで始めなくても読む。"
      >
        <List>
          <Keyed each={mailboxes() ?? []} by={(m) => m.connection}>
            {(m) => <Row of={m()} />}
          </Keyed>
        </List>
      </Section>
    </Show>
  );
}
