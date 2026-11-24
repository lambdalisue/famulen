import { createAsync } from "@solidjs/router";
import { For } from "solid-js";
import { IndexRow } from "~/components/IndexRow";
import { List } from "~/components/ui";
import * as icons from "~/lib/icons";
import { layerHref, layersQuery } from "~/lib/memory";
import { setView } from "~/lib/view";
import { AboutMemory } from "../memory";

/** The memory layers as a list of pages, for the narrow layout. */
export default function MemoryMenu() {
  setView("記憶");
  const layers = createAsync(() => layersQuery());
  return (
    <>
      <List>
        <For each={layers()}>
          {(l) => <IndexRow href={layerHref(l.id)} icon={icons.brain} name={l.label} detail={l.help} count={l.count} />}
        </For>
      </List>
      <AboutMemory />
    </>
  );
}
