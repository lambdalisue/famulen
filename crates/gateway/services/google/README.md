# Google のアプリを登録する

Gmail とカレンダーを Famulen が直接読むための、OAuth クライアントを Google Cloud に作る。作るのは一度だけで、仕事用も個人用も、このクライアントでつなぐ。サービス定義は `google.toml` の 1 つで、アカウントごとに 1 回ログインすれば、Gmail とカレンダーの両方を使える。

## 用意するもの

- アプリを持たせる Google アカウント。どのアカウントでもよい（あとでつなぐアカウントとは別でもよい）。
- コンソールの URL（`FAMULEN_CONSOLE_URL`）。戻り先は `<コンソールの URL>/oauth/callback`。

## 手順

1. Google Cloud でプロジェクトを作る（既にあれば選ぶ）。<https://console.cloud.google.com/projectcreate> 名前は `Famulen` など。
2. Gmail API を有効にする。<https://console.cloud.google.com/apis/library/gmail.googleapis.com>
3. Google Calendar API を有効にする。<https://console.cloud.google.com/apis/library/calendar-json.googleapis.com>
4. OAuth の同意画面を作る。<https://console.cloud.google.com/auth/overview>
   - アプリ名は `Famulen`、サポートのメールは自分のアドレス。
   - 対象は「外部」。
   - 連絡先のメールは自分のアドレス。
   - 「Google API サービス：ユーザーデータに関するポリシー」への同意は、アプリを持つ人が自分で行う。
5. 公開状態を決める。<https://console.cloud.google.com/auth/audience>
   - 「本番環境」にすると、ログインのとき「確認されていないアプリ」と出るが、つながりは切れない。本番にするには、ブランディングにホームページとプライバシーポリシーの URL が要る。
   - 「テスト」のままなら、つなぐアカウントを全部「テストユーザー」に入れる。テストのままだと、7 日ごとにログインし直しになる。
6. OAuth クライアントを作る。<https://console.cloud.google.com/auth/clients/create>
   - 種類は「ウェブ アプリケーション」、名前は `Famulen`。
   - 「承認済みのリダイレクト URI」に `<コンソールの URL>/oauth/callback` を入れる。コンソールを開く URL が複数あるなら、全部入れる。
7. 作ったクライアントの ID とシークレットを控える。シークレットは作った直後のダイアログでしか見られないので、「JSON をダウンロード」で控える。

## Google カレンダーの変化を受け取る

何もしなくても、カレンダーは一定の間隔で読み直される。変化をすぐ受け取りたいときだけ、`FAMULEN_RECEIVE_LISTEN` と `FAMULEN_RECEIVE_URL` を決める（`.env.sample`）。変化は、ゲートウェイの HTTP の受け口 `/receive/<つなぎ先>/calendar` に届く。Google からゲートウェイの受け口に届くようにする（Tailscale Funnel やリバースプロキシなど）かは、主人が決める。

## Famulen に渡す

```sh
FAMULEN_GOOGLE_CLIENT_ID=<クライアント ID>
FAMULEN_GOOGLE_CLIENT_SECRET=<クライアント シークレット>
```

Famulen を起動し直し、コンソールの「設定 · サービス」で「サービスを足す」から Google を選ぶ。すぐに Google のログインに進む。アカウントごとに繰り返す。

## 確かめる

- つないだアカウントがアドレスで並び、「つながっている」と出る。
- 「カレンダー」に、そのアカウントの予定が出る。
