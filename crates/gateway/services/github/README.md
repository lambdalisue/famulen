# GitHub のアプリを登録する

秘書があなたの GitHub のアカウントを使うための、OAuth アプリを作る。作るのは一度だけで、何個のアカウントでもこのアプリでつなげる。道具は GitHub 公式の MCP サーバーのもの。

GitHub App ではなく OAuth アプリを作る。GitHub App のトークンでは通知を読めず、組織ごとにインストールも要る。

## 用意するもの

- アプリを持たせる GitHub アカウント（あとでつなぐアカウントとは別でもよい）。
- コンソールの URL（`FAMULEN_CONSOLE_URL`）。

## 手順

1. <https://github.com/settings/applications/new> を開く。
2. 名前は `Famulen` など、Homepage URL は `https://github.com/lambdalisue/famulen`。
3. Redirect URI に `<コンソールの URL>/oauth/callback` を入れて登録する。コンソールを開く URL が複数あるなら、全部入れる。「Expire user access tokens」は入れたままでよい。期限の前に Famulen が更新する。
4. 「Generate a new client secret」を押し、Client ID と Client secret を控える。secret は作った直後にしか見られない。

## Famulen に渡す

```sh
FAMULEN_GITHUB_CLIENT_ID=<Client ID>
FAMULEN_GITHUB_CLIENT_SECRET=<Client secret>
```

Famulen を起動し直し、コンソールの「設定 · サービス」の「サービスを足す」で GitHub を選ぶ。アカウントごとに選んでログインする。

組織が OAuth アプリを制限しているときは、その組織のリポジトリは組織の承認があるまで読めない。ログインの画面で組織の「Request」を押して承認を求める。

## 確かめる

- つないだアカウントがログイン名で並び、「つながっている」と出る。
- 「見張り」の「すぐに足せるもの」に、GitHub の通知が出る。
