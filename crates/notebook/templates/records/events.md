---
code: event
fields:
  Status: [hold, tentative, confirmed, cancelled]
  Start: date-or-minute
  End: date-or-minute
  Timezone: text
  Calendar: link:calendars
  Busy: flag
  Response: [accepted, declined, tentative, needs_action]
  Location: text
  Map: url
  Meeting: url
  Repeat: text
sections:
  Attendees: [{Who: text, Email: text, Response: [accepted, declined, tentative, needs_action], Organizer: flag, Optional: flag}]
  Exceptions: [{At: date-or-minute, Title: text, Status: [hold, tentative, confirmed, cancelled], Start: date-or-minute, End: date-or-minute, Busy: flag, Response: [accepted, declined, tentative, needs_action], Location: text, Map: url, Meeting: url, Note: text}]
---

# 予定

見出しが予定の名前。`Start` と `End` は始まりと終わりで、両方が日付だけなら終日（`End` の日を含む）。`Status` の `hold` は仮押さえ。`Repeat` は RFC 5545 の RRULE で、`Exceptions` に、ほかの回と違う回をもとの始まりとともに書く。

Google のカレンダーにある予定は、この手帳に写さない。備考やラベルを足した予定だけがファイルになり、メタデータの `google` に予定の ID、`url` にその URL を持つ。時刻や参加者は Google のものを見る。
