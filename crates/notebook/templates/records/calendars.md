---
code: calendar
fields:
  Color: text
  Default for: list
sections:
  Show: [{Workgroup: text, Mode: [show, faint, hide]}]
---

# カレンダー

予定を分けるカレンダー。メタデータの `google`（`<つなぎ先>/<カレンダーの ID>`）があれば、その予定は Google のカレンダーにあり、この手帳には写さない。`Default for` は、新しい予定を入れる既定にするワークグループ。`Show` は、ワークグループごとの出し方。
