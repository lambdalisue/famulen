import { createAsync } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import { ARRIVED } from "~/components/LookoutInterval";
import { IntervalField, PlacesField, RouteField, WHOLE, WorkgroupField } from "~/components/LookoutFields";
import { argsOf, fieldsOf, ToolField, typedOf } from "~/components/ToolArgs";
import { type LookoutDefault, type LookoutPlace, type LookoutPreset, LookoutRoute, type LookoutSource } from "~/gen/famulen/v1/lookout_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { connectionsQuery, lookoutChoicesQuery, workspacesQuery } from "~/lib/connections";
import * as icons from "~/lib/icons";
import { DialogActions, Disclosure, Field, Select, TextField } from "~/components/ui";

/** A source of one's own: its connection, tool, arguments and interval. */
export default function LookoutForm(props: { defaults: LookoutDefault[]; presets: LookoutPreset[]; sources: LookoutSource[]; onDone: () => void }) {
  const list = createAsync(() => connectionsQuery());
  const spaces = createAsync(() => workspacesQuery());
  const [name, setName] = createSignal("");
  const [connection, setConnection] = createSignal("");
  const [tool, setTool] = createSignal("");
  const [args, setArgs] = createSignal("");
  const [advanced, setAdvanced] = createSignal(false);
  const choices = createAsync(async () => {
    const id = connection();
    return id ? { id, ...(await lookoutChoicesQuery(id)) } : undefined;
  });
  const current = () => (choices()?.id === connection() ? choices() : undefined);
  const placeLabel = () => current()?.placeLabel || "場所";
  const rest = () => props.sources.find((s) => s.connection === connection() && s.tool === ARRIVED && s.places.length === 0);
  const hasPlaces = () => (current()?.places.length ?? 0) > 0;
  const [chooses, setChooses] = createSignal(false);
  const [places, setPlaces] = createSignal<LookoutPlace[]>([]);
  const narrowed = () => hasPlaces() && (chooses() || rest() !== undefined);
  const needsTool = () => current() !== undefined && (!current()?.receives || (rest() !== undefined && !hasPlaces()));
  const fields = () => fieldsOf(current()?.tools.find((t) => t.name === tool())?.inputSchema ?? "");
  const [typed, setTyped] = createSignal<Record<string, string>>({});
  const reads = () => {
    if (!current()) return "";
    if (tool()) return `道具 ${tool()} を決まった間隔で呼んで、結果を読む。`;
    const taken = rest();
    if (taken && !hasPlaces()) return `このつなぎ先に届いたものは「${taken.name}」が見張っている。ほかに見るなら、決まった間隔で呼ぶ道具を選ぶ。`;
    return needsTool() ? "このつなぎ先からは何も届かない。決まった間隔で呼ぶ道具を選ぶ。" : "このつなぎ先に届いたものを読む。";
  };
  const chooseTool = (name: string) => {
    setTool(name);
    const preset = props.presets.find((p) => p.connection === connection() && p.tool === name);
    setArgs(preset?.args ?? "");
    setTyped(typedOf(preset?.args ?? ""));
  };
  const [route, setRoute] = createSignal(LookoutRoute.UNSPECIFIED);
  const [workgroup, setWorkgroup] = createSignal(WHOLE);
  const add = async () => {
    const known = fields();
    const reading = tool() ? { tool: tool(), args: known ? argsOf(known, typed()) : args() } : { places: narrowed() ? places() : [] };
    if (!tool() && narrowed() && places().length === 0) throw new Error(`${placeLabel()}を一つは選ぶ`);
    const added = await api.lookout.addLookout({
      name: name().trim(),
      connection: connection(),
      ...reading,
      intervalMinutes: own() ? Number.parseInt(minutes(), 10) : 0,
    });
    if (route() !== LookoutRoute.UNSPECIFIED || workgroup() !== WHOLE) {
      await api.lookout.updateLookout({
        id: added.id,
        route: route(),
        workgroup: workgroup() === WHOLE ? undefined : workgroup(),
      });
    }
    return true;
  };
  const [own, setOwn] = createSignal(false);
  const [minutes, setMinutes] = createSignal("");
  const others = () => (list()?.connections ?? []).filter((c) => !spaces()?.workspaces.some((s) => s.id === c.id));
  return (
    <form
      class="dialog-form"
      onSubmit={async (e) => {
        e.preventDefault();
        const done = await act(add, "見張りに足した");
        if (done) props.onDone();
      }}
    >
      <Field label="名前">
        <TextField name="name" required placeholder="仕事のメール" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
      </Field>
      <Field label="つなぎ先">
        <Select required value={connection()} onChange={(e) => {
            setConnection(e.currentTarget.value);
            chooseTool("");
            setPlaces([]);
            setChooses(false);
          }}>
          <option value="">選ぶ</option>
          <For each={spaces()?.workspaces ?? []}>
            {(s) => <option value={s.id}>Slack のチャネルに届く出来事 · {s.name}</option>}
          </For>
          <For each={others()}>
            {(c) => (
              <option value={c.id}>
                {c.name}
                {c.account ? ` · ${c.account}` : ""}
              </option>
            )}
          </For>
        </Select>
      </Field>
      <Show when={reads()}>
        <p class="sub lookout-reads">{reads()}</p>
      </Show>
      <Show when={!tool() && hasPlaces()}>
        <PlacesField
          label={placeLabel()}
          choices={current()?.places ?? []}
          restReader={rest()?.name}
          narrowed={narrowed()}
          setNarrowed={setChooses}
          places={places()}
          setPlaces={setPlaces}
        />
      </Show>
      <Show when={current()?.problem}>
        <p class="sub lookout-reads">つなぎ先から選べるものを読めなかった（{current()?.problem}）</p>
      </Show>
      <IntervalField defaults={props.defaults} own={own()} setOwn={setOwn} minutes={minutes()} setMinutes={setMinutes} />
      <RouteField route={route()} setRoute={setRoute} />
      <WorkgroupField workgroup={workgroup()} setWorkgroup={setWorkgroup} />
      <Disclosure open={advanced() || needsTool()} onOpenChange={setAdvanced} title="詳しい設定">
          <ToolField
            tools={current()?.tools ?? []}
            none={needsTool() ? undefined : "使わない（届いたものを読む）"}
            tool={tool()}
            chooseTool={chooseTool}
            args={args()}
            setArgs={setArgs}
            typed={typed()}
            setTyped={setTyped}
          />
        </Disclosure>
      <DialogActions submit="足す" />
    </form>
  );
}
