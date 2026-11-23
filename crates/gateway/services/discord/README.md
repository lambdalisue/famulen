# Discord をつなぐ

Discord は利用者のアカウントを自動で動かすことを禁じているので、自分の bot を作ってつなぐ。bot が入ったサーバーの、bot に見えるチャンネルだけを読める。

1. <https://discord.com/developers/applications> で「New Application」を押し、名前を付ける。
2. 「Bot」の「Privileged Gateway Intents」で「Message Content Intent」を入れる。入れないと発言の中身が空になる。
3. 「Installation」の「Install Link」を「None」にしてから、「Bot」の「Public Bot」を切る。自分だけがサーバーに入れられるようになる。
4. 「Bot」の「Reset Token」でトークンを作り、Famulen の「サービスを足す」で Discord を選んで入れる。トークンは金庫に入る。
5. 「OAuth2」の「URL Generator」で `bot` を選び、「View Channels」「Read Message History」、書かせるなら「Send Messages」を選ぶ。できた URL を開き、サーバーに入れる。サーバーの管理の権限が要る。
