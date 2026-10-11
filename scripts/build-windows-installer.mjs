import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = JSON.parse(
  readFileSync(join(projectRoot, "package.json"), "utf8"),
);
const version = packageJson.version;
const legacy = process.argv.includes('--legacy');
const nativeHost = legacy ? 'legacy' : 'rust';
const publishDir = legacy ? join(
  projectRoot,
  "windows",
  "QuranMushaf.Windows",
  "bin",
  "Release",
  "net10.0-windows10.0.19041.0",
  "win-x64",
  "publish",
) : join(projectRoot, 'outputs', 'rust-windows', 'publish');
const installerScript = join(
  projectRoot,
  "windows",
  "installer",
  "QuranMushafReader.iss",
);
const outputDir = join(projectRoot, "outputs", "releases", `v${version}${legacy ? '-legacy' : ''}`);
const outputName = `QuranMushafReader-Setup-x64-${version}.exe`;
const outputPath = join(outputDir, outputName);

if (!existsSync(join(publishDir, "QuranMushaf.Windows.exe"))) {
  throw new Error(
    `The ${nativeHost} Windows publish output is missing. Run ${legacy ? 'the legacy .NET publish command' : '`npm run publish:windows`'} first.`,
  );
}

if (legacy) {
  for (const dependency of ["coreclr.dll", "hostfxr.dll", "Microsoft.ui.xaml.dll"]) {
    if (!existsSync(join(publishDir, dependency))) {
      throw new Error(`Self-contained legacy payload is incomplete: ${dependency}. Republish the .NET host.`);
    }
  }
} else {
  const report = JSON.parse(readFileSync(join(publishDir, 'RUST-HOST-BUILD.json'), 'utf8'));
  const executableSha256 = createHash('sha256').update(readFileSync(join(publishDir, 'QuranMushaf.Windows.exe'))).digest('hex');
  if (report.nativeHost !== 'rust' || report.version !== version || report.executableSha256 !== executableSha256) {
    throw new Error('Rust host is stale or changed after publication. Run npm run publish:windows first.');
  }
}

for (const transientName of [
  "QuranMushaf.Windows.exe.WebView2",
  "Install-QuranMushafReader.cmd",
  "Uninstall-QuranMushafReader.cmd",
]) {
  rmSync(join(publishDir, transientName), { recursive: true, force: true });
}

const compilerCandidates = [
  process.env.INNO_SETUP_COMPILER,
  process.env.LOCALAPPDATA &&
    join(process.env.LOCALAPPDATA, "Programs", "Inno Setup 6", "ISCC.exe"),
  process.env["ProgramFiles(x86)"] &&
    join(process.env["ProgramFiles(x86)"], "Inno Setup 6", "ISCC.exe"),
  process.env.ProgramFiles &&
    join(process.env.ProgramFiles, "Inno Setup 6", "ISCC.exe"),
].filter(Boolean);
const compiler = compilerCandidates.find((candidate) => existsSync(candidate));

if (!compiler) {
  throw new Error(
    "Inno Setup 6 was not found. Install it or set INNO_SETUP_COMPILER to ISCC.exe.",
  );
}

mkdirSync(outputDir, { recursive: true });
const verify = spawnSync(process.execPath, [join(projectRoot, "scripts", "verify-packaged-web.mjs"), join(publishDir, 'WebApp')], {
  cwd: projectRoot, stdio: "inherit",
});
if (verify.status !== 0) throw new Error("Packaged web assets differ from the tested build. Republish before creating an installer.");
rmSync(outputPath, { force: true });
function sign(file) {
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-File',
    join(projectRoot, 'scripts', 'sign-windows-release.ps1'), '-Path', file], {cwd: projectRoot, stdio: 'inherit'});
  if (result.error || result.status !== 0) throw new Error('Release signing gate failed.');
}
sign(join(publishDir, 'QuranMushaf.Windows.exe'));
if (!legacy) {
  // Signing changes the executable bytes. Retain a manifest that verifies on a
  // subsequent installer build as well as in the portable published folder.
  const reportPath = join(publishDir, 'RUST-HOST-BUILD.json');
  const report = JSON.parse(readFileSync(reportPath, 'utf8'));
  const executable = readFileSync(join(publishDir, 'QuranMushaf.Windows.exe'));
  report.executableSha256 = createHash('sha256').update(executable).digest('hex');
  const entry = report.files.find(file => file.path === 'QuranMushaf.Windows.exe');
  if (entry) {
    report.payloadBytes += executable.length - entry.bytes;
    entry.bytes = executable.length;
    entry.sha256 = report.executableSha256;
  }
  writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

const compile = spawnSync(
  compiler,
  [
    `/DAppVersion=${version}`,
    `/DPublishDir=${publishDir}`,
    `/DOutputDir=${outputDir}`,
    `/DNativeHost=${nativeHost}`,
    installerScript,
  ],
  { cwd: projectRoot, encoding: "utf8", stdio: "inherit" },
);

if (compile.error) {
  throw compile.error;
}
if (compile.status !== 0 || !existsSync(outputPath)) {
  throw new Error(`Inno Setup failed with exit code ${compile.status}.`);
}

sign(outputPath);

const sha256 = createHash("sha256")
  .update(readFileSync(outputPath))
  .digest("hex");
writeFileSync(`${outputPath}.sha256.txt`, `${sha256}  ${outputName}\n`, "ascii");

console.log(`Created ${outputPath}`);
console.log(`SHA-256 ${sha256}`);
