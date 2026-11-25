import { createAsync, useSearchParams } from "@solidjs/router";
import { createMemo, createSignal, For, Show } from "solid-js";
import AskAbout from "~/components/AskAbout";
import UsageChart, { type Column } from "~/components/UsageChart";
import type { UsageRow } from "~/gen/famulen/v1/settings_pb";
import * as icons from "~/lib/icons";
import { modelsQuery, shortTokens, usageQuery, yen } from "~/lib/models";
import {
  type By,
  bucketName,
  colorOf,
  keyOf,
  keysOf,
  periodName,
  SPANS,
  seriesId,
  seriesOf,
  shiftDay,
  type Span,
  sum,
  tickName,
} from "~/lib/usage";
import { EmptyState, IconButton, PageHeader, SegmentedOf } from "~/components/ui";

const BY: [By, string][] = [
  ["agent", "エージェントごと"],
  ["model", "モデルごと"],
];

function plainNumber(v: number): string {
  return v.toLocaleString("ja-JP", { maximumFractionDigits: 2 });
}

/** A period's calls, tokens and cost over time, by agent or by model. */
export default function Usage() {
  const [params, setParams] = useSearchParams<{ by?: string; span?: string; day?: string }>();
  const by = (): By => (params.by === "model" ? "model" : "agent");
  const span = (): Span => SPANS.find(([key]) => key === params.span)?.[0] ?? "month";
  const usage = createAsync(() => usageQuery(SPANS.find(([key]) => key === span())![2], params.day ?? ""));
  const models = createAsync(() => modelsQuery());
  const [picked, setPicked] = createSignal<string>();

  const show = (next: { by?: string; span?: string; day?: string }) => {
    setPicked(undefined);
    setParams(next);
  };
  const model = (id: string) => [...(models()?.models ?? []), ...(models()?.candidates ?? [])].find((m) => m.id === id);
  const label = (kind: By, key: string) =>
    kind === "agent" ? (models()?.roles.find((r) => String(r.role) === key)?.label ?? "") : (model(key)?.name ?? key);
  const cost = (rows: UsageRow[]) => {
    const monthly = rows.every((r) => (model(r.model)?.monthlyYen ?? 0) > 0);
    return rows.length > 0 && monthly ? "定額" : yen(sum(rows).yen);
  };

  const total = createMemo(() => sum(usage()?.rows ?? []));
  const pickedBucket = () => usage()?.buckets.find((b) => b.from === picked());
  const pick = (key: string) => setPicked(picked() === key ? undefined : key);

  const chart = (metered: boolean) => {
    const u = usage();
    if (!u) return undefined;
    const value = (r: UsageRow) => (metered ? r.yen : Number(r.calls));
    const rows = u.rows.filter((r) => r.metered === metered && value(r) > 0);
    if (rows.length === 0) return undefined;
    const columns: Column[] = u.buckets.map((b) => {
      const values: Record<string, number> = {};
      for (const r of b.rows.filter((r) => r.metered === metered)) {
        const id = seriesId(u, by(), keyOf(r, by()));
        values[id] = (values[id] ?? 0) + value(r);
      }
      return {
        key: b.from,
        name: bucketName(span(), b.from),
        tick: tickName(span(), b.from),
        ahead: b.from > u.today,
        values,
      };
    });
    return { columns, series: seriesOf(u, by(), rows, (key) => label(by(), key)) };
  };
  const calls = createMemo(() => chart(false));
  const metered = createMemo(() => chart(true));

  const Swatch = (props: { by: By; key: string }) => (
    <Show when={usage()}>{(u) => <i class="usage-swatch" style={{ background: colorOf(u(), props.by, props.key) }} />}</Show>
  );
  const Cells = (props: { rows: UsageRow[] }) => {
    const s = () => sum(props.rows);
    return (
      <>
        <td class="mono num">{plainNumber(s().calls)} 回</td>
        <td class="mono num">{shortTokens(s().inputTokens)}</td>
        <td class="mono num">{shortTokens(s().outputTokens)}</td>
        <td class="mono num">{cost(props.rows)}</td>
      </>
    );
  };
  const Head = () => (
    <thead>
      <tr>
        <th>{by() === "agent" ? "エージェント" : "モデル"}</th>
        <th class="num">回数</th>
        <th class="num">入力</th>
        <th class="num">出力</th>
        <th class="num">金額</th>
      </tr>
    </thead>
  );
  const groups = (rows: UsageRow[]) => {
    const u = usage();
    return u ? keysOf(u, by(), rows).map((key) => ({ key, rows: rows.filter((r) => keyOf(r, by()) === key) })) : [];
  };
  const other = (): By => (by() === "agent" ? "model" : "agent");

  return (
    <section class="usage">
      <PageHeader title="使った量" />
      <div class="usage-filters">
        <SegmentedOf
          label="区切り"
          value={span()}
          items={SPANS.map(([k, text]) => [k, text] as const)}
          onChoose={(v) => show({ span: v === "month" ? undefined : v })}
        />
        <div class="usage-period">
          <IconButton
            icon={icons.chevronLeft}
            label="前へ"
            onClick={() => {
              const u = usage();
              if (u) show({ day: shiftDay(u.from, -1) });
            }}
          />
          <Show when={usage()}>{(u) => <b>{periodName(span(), u())}</b>}</Show>
          <IconButton
            icon={icons.chevronRight}
            label="次へ"
            disabled={(usage()?.to ?? "") >= (usage()?.today ?? "")}
            onClick={() => {
              const u = usage();
              if (!u) return;
              const day = shiftDay(u.to, 1);
              show({ day: day > u.today ? undefined : day });
            }}
          />
        </div>
        <SegmentedOf label="見方" value={by()} items={BY} onChoose={(v) => setParams({ by: v === "agent" ? undefined : v })} />
      </div>
      <div class="usage-totals">
        <div class="card">
          <div class="sub">呼び出し</div>
          <div class="usage-figure">{plainNumber(total().calls)} 回</div>
          <div class="sub">今日 {String(usage()?.callsToday ?? 0n)} 回</div>
        </div>
        <div class="card">
          <div class="sub">トークン（入力＋出力）</div>
          <div class="usage-figure">{shortTokens(total().inputTokens + total().outputTokens)}</div>
        </div>
        <div class="card">
          <div class="sub">従量課金</div>
          <div class="usage-figure">{yen(usage()?.meteredYen ?? 0)}</div>
          <div class="sub">ほかに定額 1 か月 {yen(usage()?.fixedMonthlyYen ?? 0)}</div>
        </div>
      </div>
      <Show when={(usage()?.rows.length ?? 0) > 0} fallback={<EmptyState>この期間はまだ使っていない</EmptyState>}>
        <Show when={calls()}>
          {(c) => (
            <UsageChart
              title="値段のないモデルの回数"
              columns={c().columns}
              series={c().series}
              format={(v) => `${plainNumber(v)} 回`}
              axis={plainNumber}
              whole
              picked={picked()}
              onPick={pick}
            />
          )}
        </Show>
        <Show when={metered()}>
          {(c) => (
            <UsageChart
              title="値段のあるモデルの額"
              columns={c().columns}
              series={c().series}
              format={(v) => `${plainNumber(v)} 円`}
              axis={(v) => `${plainNumber(v)} 円`}
              whole={false}
              picked={picked()}
              onPick={pick}
            />
          )}
        </Show>
        <Show when={pickedBucket()}>
          {(b) => (
            <section class="usage-detail">
              <div class="row">
                <h2 class="grow">{bucketName(span(), b().from)}</h2>
                <AskAbout framed what={`使った量 · ${bucketName(span(), b().from)}`} />
              </div>
              <Show when={b().rows.length > 0} fallback={<EmptyState>使っていない</EmptyState>}>
                <div class="scroll">
                  <table class="table" aria-label={`${bucketName(span(), b().from)} の使った量`}>
                    <Head />
                    <tbody>
                      <For each={groups(b().rows)}>
                        {(g) => (
                          <tr>
                            <td>
                              <Swatch by={by()} key={g.key} />
                              {label(by(), g.key)}
                            </td>
                            <Cells rows={g.rows} />
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>
              </Show>
            </section>
          )}
        </Show>
        <section class="usage-detail">
          <h2>期間の合計</h2>
          <div class="scroll">
            <table class="table" aria-label="期間の合計">
              <Head />
              <For each={groups(usage()?.rows ?? [])}>
                {(g) => (
                  <tbody>
                    <tr class="usage-group">
                      <td>
                        <Swatch by={by()} key={g.key} />
                        {label(by(), g.key)}
                      </td>
                      <Cells rows={g.rows} />
                    </tr>
                    <For each={g.rows}>
                      {(r) => (
                        <tr class="usage-part">
                          <td>{label(other(), keyOf(r, other()))}</td>
                          <Cells rows={[r]} />
                        </tr>
                      )}
                    </For>
                  </tbody>
                )}
              </For>
            </table>
          </div>
        </section>
      </Show>
    </section>
  );
}
