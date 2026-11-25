---
code: party
fields:
  Kind: [person, org, group]
  Aliases: list
  Birthday: date
  Status: [current, resting]
  Last seen: date
  Added by: [owner, secretary]
sections:
  Member of: [{To: link:parties, Role: text}]
  Relations: [{To: text, As: list, Inverse: list}]
  Accounts: [{Service: text, Handle: text, Confirmed: flag, How: text}]
---

# 相手

人と、会社・チームと、ゆるいグループ。見出しが名前、`Aliases` がほかの名前。

`Relations` の `As` は、`To`（相手か、本人を指す `me`）から見てこの人が何か、`Inverse` はその逆。関係は片側にだけ書く。`Accounts` の `Confirmed` が `no` のものは推し量っただけ。

`Status` が `resting` の人は、長く出てこないので休みにしてある。なければ `current`。`Last seen` は最後に出た日。`Added by` が `secretary` の人だけが、出てこないと休みになる。
