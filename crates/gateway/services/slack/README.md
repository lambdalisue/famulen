# Slack のアプリを登録する

秘書と Slack で話し、あなたのアカウントで Slack を読むための、Slack アプリを作る。作るのは一度だけで、ワークスペースごとに「インストール」するだけで足せる。1 回のインストールで、秘書の bot と、あなたのアカウントでの読み取りが両方入る。

## 用意するもの

- アプリを作る Slack ワークスペース（どれか 1 つ）。
- https のコンソールの URL（`FAMULEN_CONSOLE_URL`）。Slack は戻り先に https しか認めない。

## 手順

1. <https://api.slack.com/apps?new_app=1> で「From a manifest」を選び、アプリを作るワークスペースを選ぶ。
2. マニフェストに [contrib/slack-app-manifest.json](../../../../contrib/slack-app-manifest.json) の中身を貼る。`redirect_urls` の `https://CONSOLE_HOST/oauth/callback` を、`<コンソールの URL>/oauth/callback` に置き換える。
3. 「Basic Information」の「App-Level Tokens」で、スコープ `connections:write` のトークンを作る（`xapp-` で始まる）。
4. 「Basic Information」の「App Credentials」で、Client ID と Client Secret を控える。
5. 「Basic Information」の「Display Information」で、アプリのアイコンに [brand/male/avatar-512.png](../../../../brand/male/avatar-512.png) か [brand/female/avatar-512.png](../../../../brand/female/avatar-512.png) を上げる。メンバーの一覧やアプリの頁に出る。秘書の発言には、コンソールで選んだ性別のアイコンが Famulen から付く。
6. 「Manage Distribution」で、ほかのワークスペースにも入れられるよう配布を有効にする。Slack のアプリ一覧に載せる必要はない。

## Famulen に渡す

```sh
FAMULEN_SLACK_CLIENT_ID=<Client ID>
FAMULEN_SLACK_CLIENT_SECRET=<Client Secret>
FAMULEN_SLACK_APP_TOKEN=<xapp- で始まるトークン>
```

Famulen を起動し直し、コンソールの「設定 · サービス」の「サービスを足す」から Slack を選ぶ。ワークスペースを選んで許可すると、そのワークスペースが加わる。ほかのワークスペースも同じように足す。

マニフェストは、bot が受ける出来事のほかに、あなたの代わりに受ける出来事（`user_events`）も申し込む。bot がいないチャンネルであなたがメンションされた発言や、あなたへの DM は、これで届く。

すでに入れたワークスペースで、あとからマニフェストの権限が増えたときは、マニフェストを貼り直し、コンソールからそのワークスペースにログインし直す。`chat:write.customize` がないあいだは、発言はアプリのアイコンで届く。

## 確かめる

- 足したワークスペースが「つながっている」と出る。
- そのワークスペースで bot に DM すると、秘書が答える。
- bot のいないチャンネルで誰かにメンションしてもらうと、見張りに入る（コンソールの見張りの最近の判断に出る）。
