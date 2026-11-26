import { solidStart } from "@solidjs/start/config";
import { nitro } from "nitro/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [
    solidStart({ middleware: "src/middleware.ts" }),
    // Scripts and styles are compressed once at build time; pages and API answers as they go out.
    nitro({ compressPublicAssets: { gzip: true } }),
  ],
  // Lightning CSS reads @custom-media, so the widths the layout changes at are named once.
  css: { transformer: "lightningcss", lightningcss: { drafts: { customMedia: true } } },
});
