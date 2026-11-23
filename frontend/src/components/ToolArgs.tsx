import { For, Match, Show, Switch } from "solid-js";
import type { LookoutTool } from "~/gen/famulen/v1/lookout_pb";
import { Field as UiField, Select, TextField } from "~/components/ui";

type Field = { name: string; title: string; type: string; description: string; required: boolean; choices: string[] };

const record = (v: unknown): Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v) ? Object.fromEntries(Object.entries(v)) : {};

function parsed(args: string): Record<string, unknown> {
  try {
    return record(JSON.parse(args));
  } catch {
    return {};
  }
}

/** The fields a tool's JSON Schema asks for; undefined when it does not say. */
export function fieldsOf(schema: string): Field[] | undefined {
  if (!schema) return undefined;
  let shape: unknown;
  try {
    shape = JSON.parse(schema);
  } catch {
    return undefined;
  }
  const root = record(shape);
  if (root.properties === undefined && root.type !== "object") return undefined;
  const required = Array.isArray(root.required) ? root.required : [];
  const fields = Object.entries(record(root.properties)).map(([name, raw]) => {
    const p = record(raw);
    return {
      name,
      title: typeof p.title === "string" && p.title ? p.title : name,
      type: typeof p.type === "string" ? p.type : "json",
      description: typeof p.description === "string" ? p.description : "",
      required: required.includes(name),
      choices: Array.isArray(p.enum) ? p.enum.filter((c) => typeof c === "string") : [],
    };
  });
  return [...fields.filter((f) => f.required), ...fields.filter((f) => !f.required)];
}

/**
 * What was typed, as the JSON arguments the fields ask for; an empty field is
 * left out, and what `before` held beside the fields is kept.
 */
export function argsOf(fields: Field[], typed: Record<string, string>, before = "{}"): string {
  const args = parsed(before);
  for (const f of fields) delete args[f.name];
  for (const f of fields) {
    const text = (typed[f.name] ?? "").trim();
    if (!text) continue;
    if (f.type === "string") {
      args[f.name] = text;
      continue;
    }
    try {
      args[f.name] = JSON.parse(text);
    } catch {
      throw new Error(`「${f.title}」を読めない`);
    }
  }
  return JSON.stringify(args);
}

/** Arguments as what would have been typed into the fields. */
export function typedOf(args: string): Record<string, string> {
  return Object.fromEntries(Object.entries(parsed(args)).map(([k, v]) => [k, typeof v === "string" ? v : JSON.stringify(v)]));
}

/** Whether two arguments say the same, in whatever order. */
export function sameArgs(a: string, b: string): boolean {
  const sorted = (args: string) => JSON.stringify(Object.entries(parsed(args)).sort(([x], [y]) => x.localeCompare(y)));
  return a === b || (isJson(a) && isJson(b) && sorted(a) === sorted(b));
}

function isJson(text: string): boolean {
  try {
    JSON.parse(text);
    return true;
  } catch {
    return false;
  }
}

const HINTS: Record<string, string> = { array: "JSON の並び", object: "JSON", json: "JSON" };

/** One input for each argument a tool takes. */
function ToolArgs(props: { fields: Field[]; typed: Record<string, string>; onType: (name: string, value: string) => void }) {
  return (
    <For each={props.fields}>
      {(f) => {
        const value = () => props.typed[f.name] ?? "";
        return (
          <UiField label={<span classList={{ mono: f.title === f.name }} title={f.name}>
              {f.title}
            </span>} hint={<>
              {f.required ? "要る。" : "なくてもよい。"}
              {f.description}
            </>}>
            <Switch
              fallback={
                <TextField classList={{ mono: f.type in HINTS }} type={f.type === "integer" || f.type === "number" ? "number" : "text"} step={f.type === "number" ? "any" : undefined} required={f.required} placeholder={HINTS[f.type] ?? ""} value={value()} onInput={(e) => props.onType(f.name, e.currentTarget.value)} />
              }
            >
              <Match when={f.choices.length > 0 || f.type === "boolean"}>
                <Select required={f.required} value={value()} onChange={(e) => props.onType(f.name, e.currentTarget.value)}>
                  <option value="">{f.required ? "選ぶ" : "決めない"}</option>
                  <For each={f.choices.length > 0 ? f.choices : ["true", "false"]}>
                    {(c) => <option value={c}>{c === "true" ? "はい" : c === "false" ? "いいえ" : c}</option>}
                  </For>
                </Select>
              </Match>
            </Switch>
            </UiField>
        );
      }}
    </For>
  );
}

/** The tool a lookout calls, out of those that only read, and what it is given. */
export function ToolField(props: {
  tools: LookoutTool[];
  /** What choosing no tool means, when that is possible. */
  none?: string;
  tool: string;
  chooseTool: (name: string) => void;
  args: string;
  setArgs: (args: string) => void;
  typed: Record<string, string>;
  setTyped: (typed: Record<string, string>) => void;
}) {
  const chosen = () => props.tools.find((t) => t.name === props.tool);
  const names = () => {
    const listed = props.tools.map((t) => t.name);
    return props.tool && !listed.includes(props.tool) ? [props.tool, ...listed] : listed;
  };
  return (
    <>
      <UiField label="読む道具" hint={chosen()?.description ||
            "決まった間隔で呼び、結果に新しく出てきたものを見る。確認なしに呼ぶので、読むだけの道具しか選べない。"}>
        <Select class="mono" required={!props.none} onChange={(e) => props.chooseTool(e.currentTarget.value)}>
          <option value="" selected={!props.tool}>
            {props.none ?? "選ぶ"}
          </option>
          <For each={names()}>
            {(name) => (
              <option value={name} selected={name === props.tool}>
                {name}
              </option>
            )}
          </For>
        </Select>
      </UiField>
      <Show when={props.tool}>
        <Show
          when={fieldsOf(chosen()?.inputSchema ?? "")}
          fallback={
            <UiField label="条件（JSON）" hint="この道具は、何を受け取るかを言わない。要らなければ空のままでよい。">
              <TextField class="mono" placeholder='{"query":"is:unread newer_than:1d"}' value={props.args} onInput={(e) => props.setArgs(e.currentTarget.value)} />
            </UiField>
          }
        >
          {(known) => (
            <ToolArgs fields={known()} typed={props.typed} onType={(k, v) => props.setTyped({ ...props.typed, [k]: v })} />
          )}
        </Show>
      </Show>
    </>
  );
}
