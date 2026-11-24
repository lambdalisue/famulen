import { createAsync } from "@solidjs/router";
import { createSignal, Show } from "solid-js";
import { IntervalField, PlacesField, RouteField, WHOLE, WorkgroupField } from "~/components/LookoutFields";
import { ARRIVED } from "~/components/LookoutInterval";
import { argsOf, fieldsOf, sameArgs, ToolField, typedOf } from "~/components/ToolArgs";
import type { LookoutDefault, LookoutSource } from "~/gen/famulen/v1/lookout_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { lookoutChoicesQuery } from "~/lib/connections";
import * as icons from "~/lib/icons";
import { Dialog, DialogActions, DialogTrigger, Field, Icon, TextField } from "~/components/ui";

function EditForm(props: { source: LookoutSource; defaults: LookoutDefault[]; sources: LookoutSource[]; onDone: () => void }) {
  const first = props.source;
  const [name, setName] = createSignal(first.name);
  const [own, setOwn] = createSignal(first.ownInterval);
  const [minutes, setMinutes] = createSignal(String(first.intervalMinutes));
  const [route, setRoute] = createSignal(first.route);
  // One in several workgroups is taken as belonging to none of them alone.
  const before = first.workgroups.length === 1 ? (first.workgroups[0] ?? WHOLE) : WHOLE;
  const [workgroup, setWorkgroup] = createSignal(before);
  const arrived = first.tool === ARRIVED;
  const choices = createAsync(() => lookoutChoicesQuery(first.connection));
  const [narrowed, setNarrowed] = createSignal(first.places.length > 0);
  const [places, setPlaces] = createSignal(first.places);
  const rest = () =>
    props.sources.find((s) => s.id !== first.id && s.connection === first.connection && s.tool === ARRIVED && s.places.length === 0);
  const [tool, setTool] = createSignal(first.tool);
  const [args, setArgs] = createSignal(first.args);
  const [typed, setTyped] = createSignal(typedOf(first.args));
  const chooseTool = (name: string) => {
    setTool(name);
    setArgs(name === first.tool ? first.args : "");
    setTyped(name === first.tool ? typedOf(first.args) : {});
  };
  const saveReading = async () => {
    const fields = fieldsOf(choices()?.tools.find((t) => t.name === tool())?.inputSchema ?? "");
    const next = fields ? argsOf(fields, typed(), args()) : args();
    if (tool() !== first.tool || !sameArgs(next, first.args)) {
      await api.lookout.setLookoutReading({ id: first.id, tool: tool(), args: next });
    }
  };
  const save = async () => {
    if (!arrived) await saveReading();
    const chosen = narrowed() ? places() : [];
    if (narrowed() && chosen.length === 0) throw new Error(`${choices()?.placeLabel || "場所"}を一つは選ぶ`);
    if (arrived && chosen.map((p) => p.id).join() !== first.places.map((p) => p.id).join()) {
      await api.lookout.setLookoutPlaces({ id: first.id, places: chosen });
    }
    await api.lookout.updateLookout({
      id: first.id,
      name: name().trim(),
      route: route(),
      workgroup: workgroup() === before ? undefined : workgroup() === WHOLE ? "" : workgroup(),
    });
    const interval = own() ? Number.parseInt(minutes(), 10) : 0;
    if (interval !== (first.ownInterval ? first.intervalMinutes : 0)) {
      await api.lookout.setLookoutInterval({ id: first.id, intervalMinutes: interval });
    }
    return true;
  };
  return (
    <form
      class="dialog-form"
      onSubmit={async (e) => {
        e.preventDefault();
        if (await act(save, "見張りを直した")) props.onDone();
      }}
    >
      <Field label="名前">
        <TextField required value={name()} onInput={(e) => setName(e.currentTarget.value)} />
      </Field>
      <Show when={arrived && ((choices()?.places.length ?? 0) > 0 || first.places.length > 0)}>
        <PlacesField
          label={choices()?.placeLabel || "場所"}
          choices={choices()?.places ?? []}
          restReader={rest()?.name}
          narrowed={narrowed()}
          setNarrowed={setNarrowed}
          places={places()}
          setPlaces={setPlaces}
        />
      </Show>
      <Show when={!arrived}>
        <ToolField
          tools={choices()?.tools ?? []}
          tool={tool()}
          chooseTool={chooseTool}
          args={args()}
          setArgs={setArgs}
          typed={typed()}
          setTyped={setTyped}
        />
      </Show>
      <IntervalField defaults={props.defaults} own={own()} setOwn={setOwn} minutes={minutes()} setMinutes={setMinutes} />
      <RouteField route={route()} setRoute={setRoute} />
      <WorkgroupField workgroup={workgroup()} setWorkgroup={setWorkgroup} />
      <DialogActions submit="保存" />
    </form>
  );
}

/** The button that opens a source, however it was added, to change all the owner can choose. */
export default function LookoutEdit(props: { source: LookoutSource; defaults: LookoutDefault[]; sources: LookoutSource[] }) {
  const [open, setOpen] = createSignal(false);
  return (
    <Dialog
      open={open()}
      onOpenChange={setOpen}
      title="見張りを直す"
      trigger={<DialogTrigger small icon={icons.pencil} label={`${props.source.name}を直す`} />}
    >
      <EditForm source={props.source} defaults={props.defaults} sources={props.sources} onDone={() => setOpen(false)} />
    </Dialog>
  );
}
