import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL(".", import.meta.url));

export default {
  root: projectRoot,
  publicDir: "public-qul",
  resolve: {
    alias: {
      "page-flip/geometry": fileURLToPath(new URL("node_modules/page-flip/src/Flip/FlipCalculation.ts", import.meta.url)),
    },
  },
  server: {
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
};
