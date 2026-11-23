import { createMemo, For, type JSX } from "solid-js";

/**
 * A list whose rows stay while their key is listed. `For` tells rows apart by
 * the object itself, and a reload brings every object anew: it would render
 * each row again and drop what is being typed in it.
 */
export default function Keyed<T, K extends string | number | bigint>(props: {
  each: readonly T[];
  by: (item: T) => K;
  children: (item: () => T) => JSX.Element;
}) {
  const byKey = createMemo(() => new Map(props.each.map((item) => [props.by(item), item])));
  return (
    <For each={[...byKey().keys()]}>
      {(key) => {
        const first = byKey().get(key);
        if (first === undefined) return null;
        const item = createMemo<T>((last) => byKey().get(key) ?? last, first);
        return props.children(item);
      }}
    </For>
  );
}
