---
code: party
fields:
  Kind: [person, org, group]
  Aliases: list
  Birthday: date
sections:
  Member of: [{To: link:parties, Role: text}]
  Relations: [{To: text, As: list, Inverse: list}]
  Accounts: [{Service: text, Handle: text, Confirmed: flag, How: text}]
---

# 相手

人と、会社・チームと、ゆるいグループ。見出しが名前、`Aliases` がほかの名前。

`Relations` の `As` は、`To`（相手か、本人を指す `me`）から見てこの人が何か、`Inverse` はその逆。関係は片側にだけ書く。`Accounts` の `Confirmed` が `no` のものは推し量っただけ。
