import { Segmented } from "~/components/ui";
import * as icons from "~/lib/icons";
import { chooseTheme, type Theme, theme } from "~/lib/theme";

const THEMES: { value: Theme; label: string; icon: string }[] = [
  { value: "system", label: "システム", icon: icons.deviceDesktop },
  { value: "light", label: "ライト", icon: icons.sun },
  { value: "dark", label: "ダーク", icon: icons.moon },
];

/** Light, dark, or as the system has it; `brief` shows the icons alone. */
export default function ThemeChoice(props: { label: string; brief?: boolean }) {
  return (
    <Segmented
      label={props.label}
      items={THEMES.map((t) => [t.value, t.label, t.icon])}
      brief={props.brief}
      value={theme()}
      onChoose={(v) => {
        const chosen = THEMES.find((t) => t.value === v);
        if (chosen) chooseTheme(chosen.value);
      }}
    />
  );
}
