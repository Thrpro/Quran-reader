import { createServer } from "vite";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));

const server = await createServer({
  configFile: false,
  root: projectRoot,
  publicDir: "public-qul",
  server: {
    host: "127.0.0.1",
    watch: {
      ignored: [
        "**/.git/**",
        "**/.npm-cache/**",
        "**/dist/**",
        "**/outputs/**",
        "**/work/**",
        "**/windows/**/bin/**",
        "**/windows/**/obj/**",
        "**/*.WebView2/**",
      ],
    },
  },
});

await server.listen();
server.printUrls();

setInterval(() => undefined, 60_000);
