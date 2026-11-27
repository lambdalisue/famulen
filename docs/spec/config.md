# 設定

Famulen は、何も設定しないまま起動できる。環境変数（`.env` を含む）と設定ファイルは、最初の値と、動き始める前に要るものを与える。

## コンソールで決めるもの

- 秘書：名前と性別と話し方、[書ける場所](secretary.md#スキルと書ける場所)、[手帳の場所](notebook.md#手帳の場所)、[手帳の共有](notebook.md#手帳の共有)。
- 夢：[記憶](memory.md#夢)のとおり。
- 見張り：[見張り](lookout.md)のとおり。
- トリアージ：[注視の長さ](channels.md#注視)。
- モデル：使えるモデルと、役割ごとの割り当てと上限（[モデル](models.md)）。
- [ワークグループ](workgroups.md)と、どのワークグループにも効く主人のタイムゾーン。
- チャネル：話しかけを受けるか（[チャネル](channels.md)）。
- [通知](console.md#通知)：受け取る端末と、載せること。
- サービス：つなぎ先、道具の権限、カレンダー（[ゲートウェイ](gateway.md)、[道具の権限](tool-permissions.md)、[カレンダー](calendars.md)）。
- [金庫](gateway.md#金庫)。

コンソールで決めた値は、環境変数と設定ファイルの値より優先される。コンソールで決めた値を消すと、環境変数・設定ファイル・既定の値に戻る。正しくない値があれば、何も保存されない。

設定を保存すると、Famulen は関わる部分を新しい設定で組み直す。会話は続く。秘書が仕事をしている間に保存した設定は、その仕事が終わってから効く。

## 環境変数と設定ファイル

- 起動したディレクトリに `.env` があれば、まずそれを読む。すでに環境変数として設定されている名前は、`.env` の値で上書きしない。
- 設定ファイルは、`famulen <設定ファイル>` と渡したときだけ読む。TOML で書く。どの項目も省ける。知らない項目があると起動しない。
- 環境変数、設定ファイル、既定の順に優先する。空の環境変数は、設定されていないものとして扱う。
- 秘密（金庫の鍵、アプリのシークレット）は、環境変数からだけ読む。
- 読めない値があれば、その名前を示して起動しない。
- `FAMULEN_` で始まる環境変数は、Famulen が動かすほかのプログラム（モデル CLI やローカル MCP のサーバー）には渡らない。

| 環境変数 | 設定ファイル | 中身 | 既定 |
| --- | --- | --- | --- |
| `FAMULEN_CLAUDE_MODEL` | `[claude] model` | 秘書の[役割](models.md#役割)に割り当てたものがモデル名を持たないときの、Claude Code のモデル | Claude Code の既定 |
| `FAMULEN_CLAUDE_EFFORT` | `[claude] effort` | 秘書の割り当てに考える深さがないときの深さ（`low`・`medium`・`high`・`xhigh`・`max`） | Claude Code の既定 |
| （なし） | `[claude] bin` | Claude Code の実行ファイル | `claude` |
| `FAMULEN_TIMEZONE` | `timezone` | 主人のタイムゾーン（`Asia/Tokyo` などの IANA の名前） | このマシンのタイムゾーン、分からなければ UTC |
| `FAMULEN_DATA_DIR` | `data_dir` | データの置き場所 | `$XDG_DATA_HOME/famulen`（`XDG_DATA_HOME` がなければ `~/.local/share/famulen`） |
| `FAMULEN_NOTEBOOK_DIR` | `notebook_dir` | コンソールで決めるまでの、手帳の既定の場所。`~/` から書ける | データの置き場所の `notebook/` |
| `FAMULEN_WORKERS_SEALED` | `[workers] sealed` | [手代](worker.md#手代)に届かせない場所。手帳の控えなど。`~/` から書ける。環境変数では `PATH` と同じ区切りで並べる | `~/notebook` |
| `FAMULEN_WORKERS_DIRECT` | `[workers] direct` | コードの仕事を[直接](worker.md#直接)動かしてよい、一つのリポジトリ（jj のチェックアウト）。`~/` から書ける。消すと直接の委託は始まらない | なし |
| （書けない） | `[workers] direct_by_default` | 委ね先を言わないコードの仕事のうち、仕上げを許したリポジトリのものを直接にする | `false` |
| （書けない） | `[[workers.finishing]]` | [Famulen が行う仕上げ](#仕上げの許し) | なし |
| `FAMULEN_LISTEN` | `[api] listen` | API の待ち受け先 | `127.0.0.1:7790` |
| `FAMULEN_API_HOSTS` | `[api] hosts` | API がほかに答えてよいホスト名（カンマ区切り）。localhost と待ち受け先の番地には、いつも答える | なし |
| `FAMULEN_RECEIVE_LISTEN` | （書けない） | [ゲートウェイ](gateway.md#受け口と宛先)の HTTP の受け口の待ち受け先。なければ HTTP の受け口は開かない | なし |
| `FAMULEN_RECEIVE_URL` | （書けない） | ゲートウェイの受け口に外から届く URL（受け口の番地を Tailscale Funnel などで外に出したとき） | なし |
| `FAMULEN_CONSOLE_URL` | `console_url` | 主人がコンソールを開く URL。ログインの戻り先になる | なし |
| `FAMULEN_GITHUB_CLIENT_ID`・`FAMULEN_GITHUB_CLIENT_SECRET` | （書けない） | Famulen の GitHub の OAuth アプリ（[登録の手順](../../crates/gateway/services/github/README.md)） | なし |
| `FAMULEN_GOOGLE_CLIENT_ID`・`FAMULEN_GOOGLE_CLIENT_SECRET` | （書けない） | Famulen の Google アプリ（[登録の手順](../../crates/gateway/services/google/README.md)） | なし |
| `FAMULEN_SLACK_CLIENT_ID`・`FAMULEN_SLACK_CLIENT_SECRET`・`FAMULEN_SLACK_APP_TOKEN` | （書けない） | Famulen の Slack アプリと、その Socket Mode のトークン（`xapp-`）（[登録の手順](../../crates/gateway/services/slack/README.md)） | なし |
| `FAMULEN_VAULT_KEY` | （書けない） | [金庫](gateway.md#金庫)の暗号の鍵（age の X25519 の鍵、`AGE-SECRET-KEY-1…`）。なければ金庫は使えず、起動のときに新しい鍵の例が記録に出る | なし |

コンソールは別のプロセスなので、次の環境変数はコンソールに渡す。

| 環境変数 | 中身 | 既定 |
| --- | --- | --- |
| `FAMULEN_API_URL` | 本体の API の URL | `http://127.0.0.1:7790` |
| `FAMULEN_CONSOLE_URL` | 主人がコンソールを開く URL。このホスト名で開ける | なし |
| `FAMULEN_CONSOLE_HOSTS` | ほかに開いてよいホスト名（カンマ区切り） | なし |

## 仕上げの許し

`[[workers.finishing]]` は、[Famulen が仕上げてよい](worker.md#famulen-が行う仕上げ)リポジトリを一つずつ名指す。項目があると、Famulen はそのリポジトリの仕事を push して PR を開く。項目がなければ外へ何もしない。

```toml
[workers]
direct = "~/src/famulen"
direct_by_default = true

[[workers.finishing]]
repository = "~/src/famulen"
checks = ["just ci"]
reviews = 2
merge = false
deploy = "just deploy"
forge = ["gh"]
```

| キー | 意味 | 既定 |
| --- | --- | --- |
| `repository` | 直接の仕事を動かすチェックアウト | 必須 |
| `checks` | レビューの前に通らなければならないシェルのコマンド | なし |
| `reviews` | レビューする回数の上限。使い切ったあとに中身が変わると、出さずに止める。`0` でレビューしない | `2` |
| `merge` | 確認せずにマージするか | `false` |
| `deploy` | マージした中身を反映するシェルのコマンド。Famulen 自身のリポジトリでだけ使う | なし |
| `forge` | PR を扱うコマンド。`gh` の引数を受けるもの | `["gh"]` |

## テストで押さえること

- 何も設定しなくても起動できる
- 秘密（金庫の鍵、アプリのシークレット）は環境変数からだけ読まれ、設定ファイルには書けない
- 空の環境変数は、設定されていないものとして扱われる
- 環境変数は設定ファイルより、設定ファイルは既定より優先される
- 知らない考える深さやタイムゾーンは誤りになり、環境変数のときはその名前を示す
- データの置き場所の既定は `XDG_DATA_HOME`、なければホームディレクトリの下になる
- 手帳の既定の場所はデータの置き場所の外にもでき、`~` から書ける
- API の待ち受け先は環境変数でも決められ、アドレスでない値は誤りになる
- API は、localhost と待ち受け先の番地と設定したホスト名以外の Host と、ほかのサイトからの呼び出しを断る
- アプリの値が一つでも欠けていれば、そのアプリのログインは登録の手順を示して断られる
- 設定ファイルの知らない項目は誤りになる
- 最初に足したワークスペースがメインになり、メインは選び直せ、メインを外すと次のものがメインになる
- 同じワークスペースを入れ直すと、新しいワークスペースは増えず、トークンが置き換わる
- コンソールで決めた値は環境変数や設定ファイルより優先され、消すと元に戻る
- どれか一つでも正しくない値があれば、何も保存されない
- 値は形が確かめられる（モデル名、考える深さ、タイムゾーン、実行ファイル）
- コンソールで設定を保存すると、組み直しが求められる
- Slack アプリの manifest は Socket Mode を使い、公開の入口を持たない
- 金庫の鍵が age の鍵として読めなければ起動しない
- 直接動かしてよいリポジトリは、名指したときだけあり、丸ごとのパスでなければ誤りになる
- `FAMULEN_` で始まる環境変数は、Famulen が動かすほかのプログラムに渡らない
