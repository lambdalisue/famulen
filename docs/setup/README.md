# 動かす用意

コンテナで動かす前に、環境変数で渡すものを用意する。`.env.sample` を `.env` に写し、ここで用意した値を入れる。

## アプリの登録

Google や Slack、GitHub のように、つなぐ側のアプリでしかログインさせないサービスは、Famulen 用のアプリを提供元に一度だけ登録する。登録したアプリの情報は環境変数で Famulen に渡し、コンソールではアカウントを足すだけにする。1 つのアプリで、何個のアカウントでもつなげる。

ここの手順は、人が読んでも、AI に読ませて「登録して」と頼んでもそのまま進められるように書いてある。

Slack のアプリに上げるアイコンは [brand/](../../brand/README.md) にある。

| アプリ | 手順 | 渡す環境変数 |
| --- | --- | --- |
| GitHub | [github](../../crates/gateway/services/github/README.md) | `FAMULEN_GITHUB_CLIENT_ID`、`FAMULEN_GITHUB_CLIENT_SECRET` |
| Google | [google](../../crates/gateway/services/google/README.md) | `FAMULEN_GOOGLE_CLIENT_ID`、`FAMULEN_GOOGLE_CLIENT_SECRET` |
| Slack | [slack](../../crates/gateway/services/slack/README.md) | `FAMULEN_SLACK_CLIENT_ID`、`FAMULEN_SLACK_CLIENT_SECRET`、`FAMULEN_SLACK_APP_TOKEN` |

どのアプリも、ログインの戻り先にコンソールの URL を使う。`FAMULEN_CONSOLE_URL`（または設定ファイルの `console_url`）に、主人がコンソールを開く URL を入れておく。戻り先は、その URL に `/oauth/callback` を足したものになる。Slack は戻り先に https しか認めないので、Slack を使うなら https の URL にする（Tailscale の `tailscale serve` など）。

## 金庫の鍵

ログインのトークンやサービスの鍵は、ゲートウェイの金庫に暗号にして置かれる。その暗号の鍵を [age](https://age-encryption.org/) の `age-keygen` で作り、`AGE-SECRET-KEY-1` で始まる行を `.env` の `FAMULEN_VAULT_KEY` に入れる。`age-keygen` がなければ、鍵を入れずに一度起動すると、新しい鍵の例が `docker compose logs backend` に出る。鍵をなくすと、金庫の秘密は取り出せない。

## Claude Code

秘書は Claude Code として動く。コンテナの中では対話でログインできないので、ログイン済みのマシンで `claude setup-token` を実行し、出てきたトークンを `.env` の `CLAUDE_CODE_OAUTH_TOKEN` に入れる。コンテナの `/home/node/.claude` はボリュームに残るので、Claude Code の設定は起動し直しても消えない。

## 起動

リポジトリの直下で、本体とコンソールを Docker Compose で起動する。`.env` を変えたときも、同じコマンドで入れ替わる。

```sh
docker compose up -d --build
```

コンソールは `http://127.0.0.1:3000/` で開く。ほかの端末から開くときは、Tailscale Serve などで https に包み、その URL を `FAMULEN_CONSOLE_URL` に入れる。そのほかのホスト名でも開くなら、`FAMULEN_CONSOLE_HOSTS` にカンマ区切りで書く。書いていないホスト名で届いた要求は、コンソールが断る。API も同じで、コンソールの `FAMULEN_API_URL` に localhost と待ち受け先の番地のほかの名前を使うなら、その名前を本体の `FAMULEN_API_HOSTS` に書く（`compose.yaml` では書いてある）。

## ほかのマシンの AI と手帳を共有する

コンソールの設定の「秘書」の頁で「手帳の共有」を有効にすると、コンソールを開ける端末から、手帳を Git で読み書きできる（[手帳の共有](../spec/notebook.md#手帳の共有)）。コンソールを Tailscale Serve で https に包んでいれば、新しい待ち受けは要らない。

ほかのマシンには `famulen-notebook` と Git（2.38 以降）を入れ、頁に出るコマンドで手帳のコピーを作る。

```sh
cargo install --git https://github.com/lambdalisue/famulen famulen-notebook-cli
famulen-notebook clone https://<コンソールのホスト名>/notebook.git ~/notebook
```

AI には `FAMULEN_NOTEBOOK` でコピーの場所を、`FAMULEN_NOTEBOOK_AUTHOR`・`FAMULEN_NOTEBOOK_AGENT`・`FAMULEN_NOTEBOOK_MODEL` で書いた者を伝える。読む前の取り込みと、書いた後の送り出しは、コマンドがそのたびに行う。
