---
code: task
fields:
  Status: [pending, in_progress, waiting, done, cancelled]
  Assignee: text
  Priority: [high, normal, low]
  Start: date-or-minute
  Due: date-or-minute
  Finished: date-or-minute
  Waiting for: text
  Parent: link:tasks
  Blocked by: links:tasks
---

# タスク

やること。`Status` は `pending`（まだ）・`in_progress`（進めている）・`waiting`（待っている）・`done`（済んだ）・`cancelled`（やめた）。`Assignee` は `owner`（本人）・`secretary`（AI の補佐役）か、相手へのリンク。`Blocked by` は、これが済むまで進められないタスク。

題名の下のリストが決まった欄、`Details` が任意の欄、`Notes` が備考、`Log` が経過。
