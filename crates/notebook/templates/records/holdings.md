---
code: holding
fields:
  Status: [ordered, owned, lent, released]
  Category: text
  Maker: text
  Model: text
  Serial: text
  Place: text
sections:
  Acquired: {On: date, How: [bought, gift, loan, made, other], From: text, Price: money, Reason: text}
  Released: {On: date, How: [sold, given, returned, discarded, lost, other], To: text, Price: money, Reason: text}
  Deadlines: [{What: text, On: date, Reminded: flag}]
---

# 物

手元にある物と、あった物。`Acquired` が手に入れたこと、`Released` が手放したこと、`Deadlines` が保証などの期限。
