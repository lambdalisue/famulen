# 記憶

まず [core.md](core.md)（心得）を読む。いつも最初に読む決まりで、150 行まで。

| 層 | 置き場所 | 中身 |
| --- | --- | --- |
| 心得 | `core.md` | いつも最初に読む決まり |
| 保留中の判断 | `pending.md` | 根拠が足りず、決めきれなかった推し量り |
| 気づき | `episodic/YYYY-MM-DD.md` | 雑に書く受信箱。書いた日のファイルに置き、整理したら消える |
| 作業記憶 | `working/<名前>.md` | 進めている仕事の今の状態。済めば消える |
| 意味記憶 | `long/semantic/<名前>.md` | 人や好みなど、事実としての知識 |
| 手続き記憶 | `long/procedural/<名前>.md` | やり方 |
| 展望記憶 | `long/prospective/<名前>.md` | 約束や、これからすること |
| 忘却 | `forgotten/YYYY-MM.md` | 古くなった・置き換わった記憶。戻せる |

心得・保留中の判断・気づき・忘却は、1 件を 1 行に書き、行の終わりに属性を YAML の flow mapping で書く。

```markdown
- 返事は短く、要点を先に {id: m1, src: said, workgroup: [work, personal]}
```

長期記憶は 1 件を 1 ファイルに書き、欄（`id`・`summary`・`src`・`from`・`to`・`ev`・`workgroup`）を frontmatter に、記憶そのものを本文に書く。

`src` は `said`（本人が言った）・`confirmed`（本人が確かめた）・`inferred`（推し量った）のどれか。
