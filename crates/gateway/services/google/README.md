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

## Gmail の新着を受け取る（Pub/Sub）

Gmail は新着を Google Cloud の Pub/Sub のトピックに知らせ、ゲートウェイはその受け取り口から受け取る。ゲートウェイから外へつなぐだけなので、ゲートウェイの受け口を外から届くようにしなくてよい。

1. Pub/Sub API を有効にする。<https://console.cloud.google.com/apis/library/pubsub.googleapis.com>
2. トピックを作る。<https://console.cloud.google.com/cloudpubsub/topic/create> ID は `famulen-gmail` など。`projects/<プロジェクト ID>/topics/famulen-gmail` を控える。
3. Gmail がトピックへ知らせられるようにする。トピックの「権限」で、プリンシパル `gmail-api-push@system.gserviceaccount.com` に「Pub/Sub パブリッシャー」のロールを付ける。
4. トピックに受け取り口（サブスクリプション）を作る。配信のタイプは「pull」。`projects/<プロジェクト ID>/subscriptions/<名前>` を控える。
5. Gmail をつなぐアカウントに、受け取り口を読む権限を付ける。サブスクリプションの「権限」で、そのアカウントに「Pub/Sub サブスクライバー」のロールを付ける。
6. Google のアカウントをつなぐとき、トピックと受け取り口を入れる。

## Google カレンダーの変化を受け取る

カレンダーの変化は、ゲートウェイの HTTP の受け口 `/receive/<つなぎ先>/calendar` に届く。Google からゲートウェイの受け口に届くようにする（Tailscale Funnel やリバースプロキシなど）かは、主人が決める。届かなくても、一定の間隔で読み直す。

## Famulen に渡す

```sh
FAMULEN_GOOGLE_CLIENT_ID=<クライアント ID>
FAMULEN_GOOGLE_CLIENT_SECRET=<クライアント シークレット>
```

Famulen を起動し直し、コンソールの「設定 · サービス」で「サービスを足す」から Google を選ぶ。アカウントごとに選んでログインする。

## 確かめる

- つないだアカウントがアドレスで並び、「つながっている」と出る。
- 「カレンダー」に、そのアカウントの予定が出る。
