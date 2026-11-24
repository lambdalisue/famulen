import Usage from "~/components/Usage";
import { setView } from "~/lib/view";

export default function UsagePage() {
  setView("設定", "使った量");
  return <Usage />;
}
