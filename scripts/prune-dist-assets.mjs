import { rmSync } from "node:fs";
import { join } from "node:path";

const unusedRuntimeAssets = [
  "data/qul/processed/pages.json",
  "data/qul/processed/words.json",
  "experimental/digital-khatt/pages.json",
  "experimental/qpc-v2/glyph-words.json",
  "experimental/qpc-v2/layout-lines.json",
  "experimental/qpc-v2/layout-schema.json",
  "experimental/qpc-v2/pages.json",
];

const unusedRuntimeDirectories = [
  "digital-khatt-engine",
  "experimental/qpc-v2",
  "fonts/qpc-v2-pages",
];

for (const asset of unusedRuntimeAssets) {
  rmSync(join("dist", asset), { force: true });
}

for (const directory of unusedRuntimeDirectories) {
  rmSync(join("dist", directory), { recursive: true, force: true });
}

console.log(
  `Pruned ${unusedRuntimeAssets.length} source data files and ${unusedRuntimeDirectories.length} legacy directories from dist.`,
);
