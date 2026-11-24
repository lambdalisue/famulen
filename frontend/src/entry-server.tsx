// @refresh reload
import { createHandler, StartServer } from "@solidjs/start/server";
import PageIcons from "./components/PageIcons";
import { api } from "./lib/api";
import { portraitOf } from "./lib/portrait";

export default createHandler(
  (event) => (
    <StartServer
      document={({ assets, children, scripts }) => (
        <html lang="ja">
          <head>
            <meta charset="utf-8" />
            <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content" />
            <meta name="apple-mobile-web-app-capable" content="yes" />
            <meta name="mobile-web-app-capable" content="yes" />
            <meta name="apple-mobile-web-app-title" content="Famulen" />
            <PageIcons portrait={event.locals.portrait ?? portraitOf(undefined)} />
            <meta name="theme-color" content="#F4F3EE" />
            {assets}
          </head>
          <body>
            <div id="app">{children}</div>
            {scripts}
          </body>
        </html>
      )}
    />
  ),
  // The head goes out before anything a page waits for, so the portrait is
  // asked for first; the first portrait stands in while the backend is away.
  async (event) => {
    event.locals.portrait = await api.settings.getPersona({}, { timeoutMs: 2000 }).then(
      (p) => portraitOf(p.gender),
      () => undefined,
    );
    return {};
  },
);
