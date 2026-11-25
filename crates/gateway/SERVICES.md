# サービス定義の書き方

サービス定義は、[ゲートウェイ](SPEC.md)があるサービスにどうつなぐかを書いた [TOML](https://toml.io/en/v1.0.0) のファイル。ゲートウェイはサービスを名前で知らず、サービス定義に書かれた一般の部品の組み合わせのとおりに振る舞う。サービス定義には事実（どこへ、何を付けて、どう受けるか）だけを書き、届いたものの意味を読むことは書かない。

- 1 つのファイルに 1 つのサービスを書く。ファイル名は `<id>.toml`。
- ゲートウェイは、読むときに形を確かめる。知らない欄があるとき、要る欄がないとき、値の形が違うときは、そのサービス定義を読まず、わけを示す。そのサービス定義のつなぎ先は「つながらない」になる。

## 全体の形

```toml
format = 1                     # この書き方の版
id = "slack"                   # 英小文字・数字・`-`。権限の既定を引き継ぐ単位にもなる
name = "Slack"                 # 人が読む名前
about = "チャンネルの発言と DM"  # 何に使えるかの一言（なくてよい）
setup = "README.md"            # アプリの登録の手順（アプリが要るとき）。このファイルからの道か URL

[account]    # つなぎ先ごとに埋める値（要るとき）
[app]        # 登録したアプリ（要るとき）
[login]      # ログインの仕方（要るとき）
[transport]  # 通り道（1 つ）
[[tools]]    # HTTP の道具（名前を付けるとき）
[[commands]] # コマンドの道具（コマンドの通り道のとき）
[receive.<名前>]  # 受け口（要るとき。いくつでも）
[permissions]      # 権限の既定（要るとき）
```

| 欄 | 要るか | 中身 |
| --- | --- | --- |
| `format` | 要る | `1` |
| `id` | 要る | 英小文字・数字・`-` |
| `name` | 要る | 文字列 |
| `about` | | 文字列。持ち主がサービスを選ぶときに見る |
| `setup` | `[app]` があるとき | 文字列 |

## 埋め込み

文字列の値の中に `{…}` で値を埋め込める。埋め込みは、置き換えるだけで、計算や比べはしない。

| 埋め込み | 中身 | 書ける場所 |
| --- | --- | --- |
| `{account.<名前>}` | つなぎ先の値（`[account]` の欄か、ログインで得た値） | どこでも |
| `{app.<名前>}` | 登録したアプリの値 | どこでも。秘密の値は外へ送る要求の中でだけ埋まる |
| `{token}`・`{token.<名前>}` | ログインで得たトークン | 外へ送る要求の中だけ |
| `{args.<名前>}` | 道具を呼んだときの引数 | 道具の `path`・`query`・`body` |
| `{header.<名前>}`・`{body}` | 受け口に届いた要求のヘッダーと本文 | 受け口の確かめ方 |
| `{/…}` | 答えや届いたものの中の値。`/` から後は [JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901) | ログインの答え、受け口 |

- 値が埋め込み 1 つだけのとき（`"{args.limit}"`）は、指した値をそのまま使う（数や並びのまま）。ほかの文字と混ぜたときは、文字列にして埋める。
- 指した値がないとき、その欄は要求から外れる。
- `{` そのものを書くときは `{{` と書く。

## アカウント

`[account]` には、つなぎ先を足すときに持ち主が埋める欄を書く。

```toml
[account]
url = { label = "サーバーの URL" }
root = { label = "読むディレクトリ", optional = true }
display = "{account.email}"
```

- 欄の名前は英小文字・数字・`_`。`label` は持ち主に見せる名前、`optional = true` なら埋めなくてよい。
- `secret = true` の欄（合言葉など）は金庫に置かれ、外へ送るものと受け口の確かめにだけ埋まる。つなぎ先の値として読み返されることはない。
- 欄の値は、つなぎ先を足したあとにも入れ直せる。
- `{ generate = true }` と書いた欄は、持ち主に尋ねず、ゲートウェイが推測できない値をつなぎ先ごとに作って金庫に置く（受け口の合言葉など）。一度作った値は変わらず、持ち主が入れ直すこともない。ほかの欄の書き方は一緒に使えない。
- `display` は、つなぎ先の表に出すアカウントの名前。
- ログインで得た値（アカウントの ID など）も、`[login]` に書いた名前でつなぎ先の値になる。金庫には入らない。

## 登録したアプリ

```toml
[app]
id = "slack"
client_id = { env = "FAMULEN_SLACK_CLIENT_ID" }
client_secret = { env = "FAMULEN_SLACK_CLIENT_SECRET", secret = true }
signing_secret = { env = "FAMULEN_SLACK_SIGNING_SECRET", secret = true, optional = true }
app_token = { env = "FAMULEN_SLACK_APP_TOKEN", secret = true, optional = true }
```

- `id` が同じアプリは、サービス定義をまたいで 1 つ（Google のサービスを分けて書いても、同じ Google のアプリを使う）。
- ほかの欄は、アプリの値の名前と、どの環境変数から読むか。`secret = true` の値は秘密として扱い、外へ送る要求と受け口の確かめの中でだけ使う。
- `optional = true` でない値の環境変数がなければ、ログインは `setup` を示して断る。

## ログイン

`[login]` には、次のどれか 1 つを書く（使うたびに引き換えるトークンだけは、ほかに足して書く）。書かなければ、ログインは要らない。

### 鍵

```toml
[login]
key = { label = "API の鍵", header = "Authorization", value = "Bearer {token}" }
```

- 持ち主が入れた鍵は金庫に入り、`{token}` になる。`header` と `value` は、外への要求に付けるヘッダー。

### 登録したアプリでの OAuth 2.0

```toml
[login.oauth2]
authorize = "https://slack.com/oauth/v2/authorize"
token = "https://slack.com/api/oauth.v2.access"
scopes = ["chat:write", "app_mentions:read", "im:history"]
query = { user_scope = "search:read" }
tokens = { bot = "{/access_token}", user = "{/authed_user/access_token}" }
values = { team = "{/team/id}", team_name = "{/team/name}", user = "{/authed_user/id}" }
same = ["team"]
refresh = true
```

| 欄 | 要るか | 中身 |
| --- | --- | --- |
| `authorize`・`token` | 要る | [認可コードの流れ](https://www.rfc-editor.org/rfc/rfc6749#section-4.1)の 2 つの口。ゲートウェイは [PKCE](https://www.rfc-editor.org/rfc/rfc7636) を付け、client ID と secret は `[app]` から使う |
| `scopes` | | 求めるスコープの並び |
| `query` | | 認可の画面に足す値 |
| `tokens` | | token の口の答えのどこにどのトークンがあるか。名前を付けたトークンが金庫に入り、`{token.<名前>}` になる。最初に書いたものが `{token}`。なければ `access_token` の 1 つだけ |
| `values` | | 答えのどこにどのアカウントの値があるか。`{account.<名前>}` になる |
| `same` | | どの値が同じなら同じアカウントと見るか。同じアカウントをつなぎ直すと、トークンが置き換わる |
| `refresh` | | `true` なら、期限の前に更新のトークンで更新する |

答えにないアカウントの値は、ログインのあとに読む要求で得る。

```toml
[login.whoami]
get = "https://gmail.googleapis.com/gmail/v1/users/me/profile"
values = { email = "{/emailAddress}" }
```

### アカウントの値から得るトークン

```toml
[login.session]
post = "https://bsky.social/xrpc/com.atproto.server.createSession"
body = { identifier = "{account.handle}", password = "{account.app_password}" }
tokens = { access = "{/accessJwt}" }
values = { did = "{/did}" }
lifetime = "1h"
```

- つなぎ先を足したときに `body` を `post` へ送り、答えから `tokens` と `values` を得る（書き方は OAuth 2.0 と同じ）。断られたら、つなぎ先は足されない。
- `lifetime` の間だけトークンを使い、終わる前に同じ要求で取り直す。

### 使うたびに引き換えるトークン

```toml
[login.exchange.config]
post = "https://slack.com/api/tooling.tokens.rotate"
form = { refresh_token = "{account.config_refresh}" }
token = "{/token}"
renews = { config_refresh = "{/refresh_token}" }
```

- OAuth 2.0 か、アカウントの値から得るトークンのログインに足して書く。名前（`config`）を道具の `auth` に書くと、その道具を呼ぶ直前に `form` を `post` へ送り、答えの `token` にあるトークンを付けて呼ぶ。そのトークンは残さない。
- `renews` は、引き換えで使い終わるアカウントの欄と、答えのどこに次の値があるか。欄は `secret = true` でなければならず、次の値は金庫に入る。同じ値を 2 度引き換えに出すことはなく、道具には埋め込めない。
- 欄に値がないか、引き換えを断られたら、道具は呼ばれず、わけを示す。

### MCP の認可の手順

```toml
[login]
mcp = true
```

- リモート MCP の通り道で、サーバーが [MCP の Authorization](https://modelcontextprotocol.io/specification/latest/basic/authorization) に従うとき。ゲートウェイは、認可サーバーを見つけ、クライアントを登録し、PKCE でトークンを受け取る。`[app]` は要らない。

## 通り道

`[transport]` には、次のどれか 1 つを書く。

```toml
[transport]
mcp_local = { command = "files-mcp", args = ["--root", "{account.root}"], env = { LOG = "warn" } }
```

```toml
[transport]
mcp_remote = { url = "{account.url}", headers = { X-Toolsets = "repos" } }
```

```toml
[transport]
http = { base = "https://slack.com/api", auth = "bot", openapi = "https://…/openapi.json" }
```

- `mcp_remote` の `headers` は、要求に付けるヘッダー（なくてよい）。ログインのトークンも、HTTP の通り道と同じ付け方で付く。
- MCP の通り道では、道具はサーバーの `tools/list` の答え。読むだけかどうかは、道具の `readOnlyHint` による。
- HTTP の通り道では、ログインのトークンを付けて要求を送り、答えをそのまま返す。`auth` は、どのトークンを付けるか（`tokens` の名前）。付け方は、鍵なら `[login]` の `header` と `value`、OAuth 2.0 なら `Authorization: Bearer`。
- HTTP の道具は、`openapi` を書けば説明の `operationId` のある操作ごとに 1 つ（`GET` と `HEAD` は読むだけ）、`[[tools]]` を書けばその道具、どちらもなければ `http_get`・`http_post`・`http_put`・`http_patch`・`http_delete`（引数は `path`・`query`・`body`）になる。
- `openapi` は、JSON で書かれた [OpenAPI 3](https://spec.openapis.org/oas/latest.html) の説明の URL か、このファイルからの道。埋め込みを書ける。URL なら、ログインのトークンを付けて読む。
- 説明から作る道具の名前は、`operationId` を英小文字と `_` にしたもの（`listPets` は `list_pets`）。引数は、道（`path`）と問い（`query`）の引数をその名前で、JSON の本文を `body` で渡す。`[[tools]]` に同じ名前の道具があれば、そちらを使う。

```toml
[transport.command]
program = "adroit"
env = { LANG = "C" }
unset = ["ADROIT_DELEGATION_TOKEN"]
timeout = "2m"

[[commands]]
name = "start_session"
description = "Start a session."
args = ["session", "start", "--dir", "{args.dir}", ["--name", "{args.name}"], { flag = "--worktree", when = "{args.worktree}" }, "--", "{args.prompt}"]
input = { type = "object", properties = { dir = { type = "string" }, prompt = { type = "string" }, name = { type = "string" }, worktree = { type = "boolean" } }, required = ["dir", "prompt"] }
open_world = true
```

- コマンドの通り道では、道具は `[[commands]]`。呼ばれるたびに `program` を、シェルを通さずに `args` を付けて動かし、出したものを答えにする。0 でない終わり方は失敗になる。
- `args` の 1 つは、引数 1 つ（値がなければ断る）、並び（中の値が 1 つでもなければ並びごと外れる）、`flag`（`when` が `true` のときだけ付く）のどれか。埋めた値が 2 つの引数になることはない。
- `stdin` は、標準入力に渡す文字。`{ flag = "--reply", each = "{args.replies}" }` は、並びの 1 つごとに `flag` を付けて渡す。
- `each_time` を書いた道具は、権限に何を選んでも、呼ばれるたびに持ち主に尋ねる。`reads` の読むだけの道具を同じ引数で呼んだ答えの `at` の場所を、何を決めるのかとして持ち主に見せる。`same` の場所の値が引数と違えば断る。承認したら、呼んだ者に代わってゲートウェイが 1 回だけ呼び、却下したら `on_reject` の引数で呼ぶ（なければ何もしない）。`questions` を書くと、決めるものの中のその場所にある質問（`question`・`options` の `label` と `description`・`multiSelect`）に、持ち主が承認の代わりに選ぶか自分の言葉で答え、その答えが `fills` の引数に入る。「今後は常に許可」は選べない。
- `unset` は、ゲートウェイの環境からコマンドに渡さない変数。`timeout` を過ぎたら止める（なければ 1 分）。
- `read_only`・`destructive`・`open_world` の意味は HTTP の道具と同じで、どれもなければ書く道具になる。
- ゲートウェイを通して出した変更では、尋ねずに動くコマンドの動かすものを変えられない。

### HTTP の道具

```toml
[[tools]]
name = "post_message"
description = "Post a message to a channel or a thread."
method = "POST"
path = "/chat.postMessage"
body = { channel = "{args.channel}", text = "{args.text}", thread_ts = "{args.thread}" }
input = { type = "object", properties = { channel = { type = "string" }, text = { type = "string" }, thread = { type = "string" } }, required = ["channel", "text"] }
auth = "bot"
```

| 欄 | 要るか | 中身 |
| --- | --- | --- |
| `name` | 要る | 英小文字・数字・`_` |
| `description` | 要る | 道具の説明。呼ぶ者に見せる |
| `method` | 要る | `GET`・`POST`・`PUT`・`PATCH`・`DELETE` |
| `path` | 要る | `base` からの道。埋め込みを書ける。空なら `base` そのもの |
| `query`・`body` | | 値の表。埋め込みを書ける |
| `input` | | 引数の [JSON Schema](https://json-schema.org/)。なければ引数なし。引数ごとの `title` と `description` は、持ち主が引数を入れる欄の名前と説明になる |
| `read_only` | | 読むだけか。なければ `GET` だけが読むだけ |
| `destructive` | | 取り返しがつかないか。なければ `DELETE` だけが取り返しがつかない。そうなら、自動の権限でも必ず確かめる |
| `open_world` | | 外の人を巻き込むか（メッセージや招待を送るなど）。そうなら、自動の権限でも必ず確かめる |
| `auth` | | `[transport]` の `auth` と違うトークンを付けるとき |
| `same_host` | | `true` なら、`path` は URL で、`base` と同じホストの文字の答えだけを読む。別のホストの URL、別のホストへのリダイレクト、文字でない答えは失敗になる。`http` から `https` へは移れる |
| `file` | | `true` なら、答えをファイルとしてそのまま返す。`GET` の道具だけで、`ok`・`same_host` とは一緒に書けない。頼んだホストの外へのリダイレクト、成功でない状態、20 MiB を超える答えは失敗になる |
| `opens` | | 答えの中のリンクを読む道具の名前。引数 `url` を取る、`same_host` の `GET` の道具 |
| `ok` | | 答えのどこが成否か（`"{/ok}"`）。そこが `true` でなければ失敗として返す。なければ HTTP の状態が 400 より小さいとき成功 |

## 受け口

受け口は `[receive.<名前>]` に、名前を付けて書く。1 つのサービス定義に、いくつ書いてもよい（Google の 1 つのアカウントに、メールの知らせとカレンダーの知らせが届く）。名前は英小文字・数字・`_`。

- 受け口には、届いたものについて受け取る側に伝えることを `note` に書ける。
- 1 つの受け口には、HTTP の受け口（`http`）か、ゲートウェイから張る接続（`connect`）の、どちらか 1 つを書く。
- ゲートウェイは届いたものに手を加えず、つなぎ先と受け口ごとに決めた宛先へ渡す。届いたものには、どのつなぎ先のどの受け口に届いたかが付く。宛先の決まっていない受け口は開かない。
- 受け口には、勧める宛先の名前を `destination` に書ける。宛先の名前はゲートウェイを使う側が決めるもので、ゲートウェイはそれを使う側に伝えるだけで、自分では選ばない。使う側は、まだ宛先を選んでいない受け口に、その名前の宛先があれば勧めに従える。

```toml
[receive.events]
destination = "channel"

[receive.events.connect]
protocol = "slack-socket-mode"
token = "{app.app_token}"
```

### HTTP の受け口

```toml
[receive.events.http]
verify = { hmac_sha256 = { secret = "{app.signing_secret}", base = "v0:{header.X-Slack-Request-Timestamp}:{body}", signature = "{header.X-Slack-Signature}", prefix = "v0=", timestamp = "{header.X-Slack-Request-Timestamp}", max_age = "5m" } }
challenge = "{/challenge}"
```

- ゲートウェイは、`<受け口の番地>/receive/<つなぎ先>/<受け口の名前>` で受ける。番地はゲートウェイを使う側が決める。
- `verify` は、届いたものの確かめ方。次のどれか 1 つ。確かめられないものは断る。

| 確かめ方 | 中身 |
| --- | --- |
| `hmac_sha256` | `base` を `secret` で HMAC-SHA256 にかけた 16 進が、`signature`（`prefix` を除いたもの）と同じか。`timestamp` が `max_age` より古ければ断る |
| `token` | `{ value = "{header.X-Goog-Channel-Token}", equals = "{account.channel_token}" }`：届いた値が、決めた値と同じか |
| `jwt` | `{ value = "{header.Authorization}", issuer = "https://accounts.google.com", audience = "{account.receive_url}" }`：Bearer の [JWT](https://www.rfc-editor.org/rfc/rfc7519) の署名・発行元・宛先・期限が正しいか。署名の鍵は発行元の公開の鍵で確かめる |

- `challenge` を書くと、届いた本文にその値があるとき、ゲートウェイはその値をそのまま答えとして返し、宛先には渡さない（受け口の URL を確かめる問い）。

### ゲートウェイから張る接続

```toml
[receive.events.connect]
protocol = "slack-socket-mode"
token = "{app.app_token}"
account = { team = "{/payload/team_id}" }
```

```toml
[receive.mail.connect]
protocol = "pubsub-pull"
subscription = "{account.subscription}"
auth = "user"
```

| 手順 | 中身 |
| --- | --- |
| `slack-socket-mode` | [Socket Mode](https://api.slack.com/apis/socket-mode)。`token` で接続の URL を得て WebSocket でつなぎ、届いた出来事ごとに受け取りを返す |
| `command-lines` | コマンドの通り道の `program` を `args` で動かし続け、出した JSON の 1 行ごとに受け取る。`resume` を書くと、最後に受け取った行から埋めた引数を足して、次はそこから続ける。`when` を書くと、どの場所の値も挙げたもののどれかである行だけを渡す。`settle` と `per` を書くと、`per` の値が同じ行がその間ほかに来なかったときにだけ渡す。`away` も書くと、持ち主がそれだけの間マシンに触れていないか、画面をロックしているあいだは、待たずに渡す（分からないマシンでは触れていないとみなす）。`front` も書くと、そのアプリ（macOS の bundle id）が最前面にないあいだも待たずに渡す。`settle` のある受け口は、渡したあとで `per` の値が同じ、`when` に合わない行が来ると、もう当てはまらないことも渡す |
| `pubsub-pull` | Google Cloud の [Pub/Sub](https://cloud.google.com/pubsub/docs/pull) の `subscription` から、`auth` のトークンで受け取り、受け取りを返す |

- `account` は、届いたもののどこが、どのアカウントの値か。同じ接続に乗るつなぎ先のうち、その値が同じつなぎ先のものになる。場所が届いたものによって違うときは並びで書き、最初に値のある場所を使う。書かなければ、つなぎ先ごとに接続を張る。
- 同じ `token` の接続を使うつなぎ先が複数あるとき、ゲートウェイは接続を 1 つだけ張る。
- つなぎ先や宛先が変われば、ゲートウェイは張る接続をそれに合わせる。張ったままの接続は張り直さない。

## 権限の既定

```toml
[permissions]
post_message = "confirm"
search_messages = "allow"
```

- 道具ごとの権限の既定（`allow`・`confirm`・`forbid`）。持ち主がつなぎ先や種類で決めた権限のほうが優先する。書かなければ自動。
- ゲートウェイを通して出した変更（[ゲートウェイ](SPEC.md) のサービス定義の変更）では、`GET` でない道具を `allow` にできず、今の定義より緩めることもできない。使うたびに引き換えるトークンを付ける道具は、`confirm` か `forbid` でなければ足せず、変えられない。

## 持ち主でない呼ぶ者が足せる定義

持ち主でない呼ぶ者（Famulen では秘書）は、次のすべてに当てはまる定義のつなぎ先だけを足せる。ほかの定義のつなぎ先は、持ち主が足す。

- `[login]` がない。
- `[app]` がない。
- `[account]` に、`secret = true` の欄も `generate = true` の欄もない。埋めなくてよい欄でも同じ。
- 通り道が `http` で、`openapi` がない。マシンでプログラムを動かす通り道と、MCP の通り道は当てはまらない。
- `[receive.…]` がない。

値は `[account]` の欄に沿い、値を埋めた `base` が公開の `https://` の URL にならなければ断られる（[ゲートウェイの仕様](SPEC.md#鍵の要らないつなぎ先)）。

## テストで押さえること

- 持ち主でない呼ぶ者が足せるのは、鍵も秘密も要らず HTTP でだけ届く定義で、同梱の定義では RSS だけ

- 知らない欄、要る欄の抜け、形の違う値のあるサービス定義は読まれず、わけが示される
- 埋め込みは置き換えるだけで、秘密の値は外への要求と受け口の確かめの中でだけ埋まる
- 指した値がない埋め込みのある欄は、要求から外れる
- 同じ `id` のアプリは、サービス定義をまたいで 1 つになる
- OAuth 2.0 の答えから、名前を付けたトークンとアカウントの値が取り出され、トークンは金庫に入る
- `same` の値が同じアカウントをつなぎ直すと、トークンが置き換わり、持ち主が入れた秘密の値は残る
- 使うたびに引き換えるトークンは残らず、使い終わった値は金庫で次の値に替わり、2 度出されない
- HTTP の通り道は、`openapi`・`[[tools]]`・どちらもないときの、それぞれの道具を出し、`ok` が `true` でない答えは失敗になる
- `same_host` の道具は、リダイレクトされても `base` と同じホストの外へは要求を送らない
- `file` の道具は、答えを欠けずに返し、頼んだホストの外へは送られず、大きすぎる答えと失敗の状態を断る
- コマンドの道具は、値がどんな文字でも 1 つの引数として渡し、失敗と時間切れをわけとともに返す
- 受け口には名前があり、1 つのサービス定義に複数書け、受け口ごとに別の宛先へ渡る
- 宛先の決まっていない受け口と、サービス定義にない名前の受け口は開かない
- 受け口の勧める宛先は使う側に伝わるだけで、勧めただけの受け口は開かない
- HTTP の受け口は、`hmac_sha256`・`token`・`jwt` のどれでも、合わないものと古すぎるものを断る
- `challenge` の値のある本文には、その値を返し、宛先には渡さない
- ゲートウェイから張る接続は、同じ `token` なら 1 つで、届いたものは `account` の値でつなぎ先に分けられる
- ゲートウェイから張る接続は、宛先が決まると張られ、外れると閉じる
- `[permissions]` の既定は、持ち主の決めた権限より弱い
