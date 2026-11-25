import ThemeChoice from "~/components/ThemeChoice";
import { PageHeader, Section } from "~/components/ui";
import { setView } from "~/lib/view";

/** How the console looks on this device. */
export default function LookPage() {
  setView("設定", "表示");
  return (
    <>
      <PageHeader title="表示" note="この端末での見え方。選んだものはこのブラウザに残り、開いているほかのタブにも効く。" />
      <Section title="テーマ" label="テーマ" note="システムを選ぶと、端末のライトとダークに従い、端末で切り替えるとその場で替わる。">
        <ThemeChoice label="テーマ" />
      </Section>
    </>
  );
}
