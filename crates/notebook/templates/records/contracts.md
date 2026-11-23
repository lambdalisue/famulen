---
code: contract
fields:
  Status: [planned, active, ended]
  Category: text
  Provider: text
  Plan: text
  Number: text
  Started: date
  Ended: date
  Auto renew: flag
  Price: money
  Cycle: [monthly, yearly, once]
  Payment: text
  Covers: links:holdings
sections:
  Deadlines: [{What: text, On: date, Reminded: flag}]
---

# 契約

ライセンス・サブスクリプション・回線や保険。`Deadlines` が更新日や契約の切れ目。カードや口座の番号は書かない。
