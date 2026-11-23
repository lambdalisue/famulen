---
code: routine
fields:
  Schedule: list
  Timezone: text
  Start: date-or-minute
  Until: date-or-minute
  Enabled: flag
  Assignee: text
  Managed by: [owner, secretary]
  Skip: list
sections:
  Skip if: [{When: text, Status: list}]
---

# 定期タスク

決まった時刻に繰り返すタスク。`Schedule` は cron の 5 欄（分 時 日 月 曜日）、`Notes` はそのたびに何をするか、`Skip` は飛ばす回の時刻。

`Skip if` は回を飛ばす条件。項目は、タスクへのリンク（`Status` に並べた状態のどれかにあるあいだ）か、`previous unfinished`（前の回がまだ済んでいないあいだ）。
