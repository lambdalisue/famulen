import { Tooltip } from "@ark-ui/solid/tooltip";
import { For, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { type Series, ticks } from "~/lib/usage";

export interface Column {
  key: string;
  name: string;
  tick: string;
  /** Not come yet: nothing to show or pick. */
  ahead: boolean;
  /** By the id of the series. */
  values: Record<string, number>;
}

function total(column: Column): number {
  return Object.values(column.values).reduce((a, b) => a + b, 0);
}

/** One measure over time, a stacked column per day or month. */
export default function UsageChart(props: {
  title: string;
  columns: Column[];
  series: Series[];
  format: (value: number) => string;
  axis: (value: number) => string;
  whole: boolean;
  picked: string | undefined;
  onPick: (key: string) => void;
}) {
  const scale = () => ticks(Math.max(...props.columns.map(total)), props.whole);
  const top = () => scale().at(-1) ?? 1;
  const percent = (value: number) => `${(value / top()) * 100}%`;
  const used = (column: Column) => props.series.filter((s) => (column.values[s.id] ?? 0) > 0);
  const name = (column: Column) => `${column.name} · ${props.format(total(column))}`;

  return (
    <section class="card usage-chart" aria-label={props.title}>
      <div class="usage-chart-head">
        <h2>{props.title}</h2>
        <ul class="usage-legend">
          <For each={props.series}>
            {(s) => (
              <li>
                <i class="usage-swatch" style={{ background: s.color }} />
                {s.label}
              </li>
            )}
          </For>
        </ul>
      </div>
      <div class="usage-plot">
        <div class="usage-axis" aria-hidden="true">
          <For each={scale()}>{(v) => <span style={{ bottom: percent(v) }}>{props.axis(v)}</span>}</For>
        </div>
        <div class="usage-grid" aria-hidden="true">
          <For each={scale()}>{(v) => <span style={{ bottom: percent(v) }} />}</For>
        </div>
        <div
          class="usage-columns"
          style={{ "grid-template-columns": `repeat(${props.columns.length},minmax(0,1fr))` }}
        >
          <For each={props.columns}>
            {(column) => (
              <Tooltip.Root lazyMount unmountOnExit openDelay={80} closeDelay={0} closeOnClick={false} positioning={{ placement: "top" }}>
                <Tooltip.Trigger
                  class="usage-column"
                  aria-label={name(column)}
                  aria-pressed={props.picked === column.key}
                  disabled={column.ahead}
                  onClick={() => props.onPick(column.key)}
                >
                  <span class="usage-stack" style={{ height: percent(total(column)) }}>
                    <For each={used(column)}>
                      {(s) => <span style={{ "flex-grow": (column.values[s.id] / total(column)) * 100, background: s.color }} />}
                    </For>
                  </span>
                </Tooltip.Trigger>
                <Portal>
                  <Tooltip.Positioner>
                    <Tooltip.Content class="usage-tip">
                      <span class="sub">{column.name}</span>
                      <b>{props.format(total(column))}</b>
                      <For each={used(column)}>
                        {(s) => (
                          <span class="usage-tip-row">
                            <i style={{ background: s.color }} />
                            <b>{props.format(column.values[s.id])}</b>
                            <span>{s.label}</span>
                          </span>
                        )}
                      </For>
                    </Tooltip.Content>
                  </Tooltip.Positioner>
                </Portal>
              </Tooltip.Root>
            )}
          </For>
        </div>
        <div class="usage-ticks" aria-hidden="true" style={{ "grid-template-columns": `repeat(${props.columns.length},minmax(0,1fr))` }}>
          <For each={props.columns}>
            {(column) => (
              <span>
                <Show when={column.tick}>{(tick) => <span>{tick()}</span>}</Show>
              </span>
            )}
          </For>
        </div>
      </div>
    </section>
  );
}
