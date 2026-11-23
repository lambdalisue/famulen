import { Combobox, useListCollection } from "@ark-ui/solid/combobox";
import { For, Show, createSignal, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import ZoneMap from "~/components/ZoneMap";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { chevronDown, mapPin } from "~/lib/icons";
import { type Zone, allZones, browserZone, zone } from "~/lib/zones";
import { Button, Icon, Section } from "~/components/ui";

/** The owner's time zone, found by name or picked on the world map. */
export default function ZoneSection(props: { timezone: string }) {
  const zones = allZones();
  const label = (z: Zone) => `${z.id}　${z.name}（${z.offset}）`;
  const { collection, filter } = useListCollection<Zone>({
    initialItems: zones,
    itemToString: label,
    itemToValue: (z) => z.id,
    filter: (text, query) => text.toLowerCase().includes(query.toLowerCase()),
    limit: 60,
  });
  // What is typed only narrows the list; the chosen zone shows above it.
  const [query, setQuery] = createSignal("");
  const search = (text: string) => {
    setQuery(text);
    filter(text);
  };
  const [detected, setDetected] = createSignal<string>();
  onMount(() => setDetected(browserZone()));
  const current = () => zone(props.timezone);
  const pick = (id: string) => {
    if (id && id !== props.timezone) void act(() => api.settings.updatePersona({ timezone: id }));
  };
  return (
    <Section title="タイムゾーン" level={3} note="予定・起床予約・定期の仕事・夢の時刻は、このタイムゾーンで数える。">
      <div class="zone-now">
        <Icon svg={mapPin} size={16} />
        <b>{current().name || current().id}</b>
        <span class="sub">
          {current().id}（{current().offset}）
        </span>
      </div>
      <div class="row">
        <Combobox.Root
          class="zone-box"
          collection={collection()}
          value={[props.timezone]}
          inputValue={query()}
          onInputValueChange={(d) => search(d.inputValue)}
          onOpenChange={(d) => {
            if (!d.open) search("");
          }}
          onValueChange={(d) => {
            pick(d.value[0] ?? "");
            search("");
          }}
          openOnClick
        >
          <Combobox.Control class="zone-control">
            <Combobox.Input class="input" placeholder="都市や地域で探す（Tokyo、日本、New_York …）" aria-label="タイムゾーンを探す" />
            <Combobox.Trigger class="zone-trigger" aria-label="一覧を開く">
              <Icon svg={chevronDown} size={16} />
            </Combobox.Trigger>
          </Combobox.Control>
          <Portal>
            <Combobox.Positioner>
              <Combobox.Content class="zone-list">
                <Combobox.Empty class="sub zone-empty">見つからない</Combobox.Empty>
                <For each={collection().items}>
                  {(z) => (
                    <Combobox.Item item={z} class="zone-item">
                      <Combobox.ItemText>
                        <b>{z.id}</b> <span class="sub">{z.name}（{z.offset}）</span>
                      </Combobox.ItemText>
                    </Combobox.Item>
                  )}
                </For>
              </Combobox.Content>
            </Combobox.Positioner>
          </Portal>
        </Combobox.Root>
        <Show when={detected() && detected() !== props.timezone}>
          <Button small onClick={() => pick(detected() ?? "")}>
            このブラウザのタイムゾーン（{detected()}）にする
          </Button>
        </Show>
      </div>
      <ZoneMap value={props.timezone} onPick={pick} />
    </Section>
  );
}
