import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium } from "playwright";

// Connect only to a separately launched QA host. Remote debugging is compiled
// behind the Rust host's `qa` feature and uses a separate WebView2 profile.
const portIndex = process.argv.indexOf("--port");
const port = Number(portIndex >= 0 ? process.argv[portIndex + 1] : 9235);
assert.ok(Number.isInteger(port) && port > 1024 && port <= 65535, "Invalid local QA port");
const checkPersistence = process.argv.includes("--check-persistence");
const output = resolve("outputs/rust-host-qa");
const settingsKey = "quran-mushaf-qul-reader:v1";
const persistenceKey = "quran-rust-host-qa-persistence";
const cacheName = "quran-rust-host-qa";
const origin = "https://quran-mushaf.local";
const report = { checks: [], consoleErrors: [], viewports: [], origin, port,
  restartPersistence: checkPersistence, passed: false };
const reportPath = resolve(output, checkPersistence ? "native-restart-verification.json" : "native-verification.json");
await mkdir(output, { recursive: true });
let browser;
try {
  const connectionDeadline = Date.now() + 30000;
  for (;;) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 2000 });
      break;
    } catch (error) {
      if (Date.now() >= connectionDeadline) throw error;
      await new Promise(done => setTimeout(done, 500));
    }
  }
  const context = browser.contexts()[0];
  assert.ok(context, "The native WebView2 context was not found");
  const page = context.pages().find(item => item.url().startsWith(`${origin}/`))
    ?? context.pages()[0] ?? await context.waitForEvent("page", { timeout: 30000 });
  assert.ok(page, "The native Quran reader was not found at its preserved origin");
  await page.waitForURL(`${origin}/**`, { timeout: 30000 });
  assert.equal(await page.evaluate(() => window.__QURAN_RUST_HOST__), true, "This is not the Rust host");
  page.on("pageerror", error => report.consoleErrors.push(error.message));
  // Wait for React's initial preference effect before seeding or reading data.
  await page.locator(".pageSlot > .qulPage .ayahRun").first().waitFor({ timeout: 30000 });

  if (checkPersistence) {
    const saved = await page.evaluate(async ({ persistenceKey, cacheName, settingsKey }) => {
      const cache = await caches.open(cacheName);
      const response = await cache.match("/rust-host-qa-persistence");
      return { marker: localStorage.getItem(persistenceKey), cache: response ? await response.text() : null,
        theme: JSON.parse(localStorage.getItem(settingsKey) ?? "{}").theme };
    }, { persistenceKey, cacheName, settingsKey });
    assert.equal(saved.marker, "rust-profile-preserved");
    assert.equal(saved.cache, "offline-data-preserved");
    assert.equal(saved.theme, "sepia");
    report.checks.push("Settings and offline Cache API data survived a native process restart");
  }

  await page.evaluate(({ settingsKey }) => {
    localStorage.setItem("quran-reader-onboarding-v2", "done");
    localStorage.setItem(settingsKey, JSON.stringify({ settingsVersion: 5, languageSetupVersion: 1,
      hasChosenLanguage: true, page: 45, mode: "spread", theme: "dark", metadataLanguage: "english",
      sizePreset: "custom", textScale: 145, textMode: "natural", contextualText: false,
      showClock: true, clockFormat: "24", searchShortcut: "ctrl-f" }));
  }, { settingsKey });
  await page.reload();
  await page.locator(".pageSlot > .qulPage .ayahRun").first().waitFor({ timeout: 30000 });
  await page.locator(".rustTitleBar").waitFor();
  await page.waitForFunction(() => document.fonts.status === "loaded");

  async function assertViewport(stage) {
    await page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
    const viewport = await page.evaluate(() => ({ width: innerWidth, height: innerHeight }));
    report.viewports.push({ stage, ...viewport });
    assert.ok(viewport.width >= 720 && viewport.height >= 480,
      `${stage} native viewport is too small: ${viewport.width} × ${viewport.height}`);
  }
  await assertViewport("initial");

  const geometry = await page.evaluate(() => {
    const title = document.querySelector(".rustTitleBar").getBoundingClientRect();
    const reader = document.querySelector(".readerViewport").getBoundingClientRect();
    const options = document.querySelector(".rustTitleOptions").getBoundingClientRect();
    return { titleHeight: title.height, titleBottom: title.bottom, readerTop: reader.top,
      optionsLeft: options.left, origin: location.origin };
  });
  assert.equal(geometry.origin, origin);
  assert.equal(geometry.titleHeight, 36);
  assert.ok(geometry.readerTop >= geometry.titleBottom, "The title bar overlaps the Quran viewport");
  report.geometry = geometry;
  report.checks.push("Preserved HTTPS origin, rendered Quran pages and separate 36 DIP title bar with labeled tools");

  const options = page.locator(".rustTitleOptions");
  async function revealOptions() {
    if (await options.getAttribute("aria-expanded") !== "true") await options.click();
  }
  const controlsInitiallyOpen = await options.getAttribute("aria-expanded");
  await options.click();
  await page.waitForFunction(open => document.querySelector(".rustTitleOptions").getAttribute("aria-expanded") !== open, controlsInitiallyOpen);
  await options.click();
  report.checks.push("Accessible Search/options toggles reader controls");

  await page.getByRole("button", { name: "Maximize", exact: true }).click();
  await page.getByRole("button", { name: "Restore window", exact: true }).waitFor();
  await assertViewport("maximized");
  await page.getByRole("button", { name: "Restore window", exact: true }).click();
  await page.getByRole("button", { name: "Maximize", exact: true }).waitFor();
  await assertViewport("restored after maximize");
  report.checks.push("Native maximize and restore acknowledge their real window state");

  const pageNumbers = () => page.locator(".pageSlot > .qulPage").evaluateAll(items => items.map(item => item.getAttribute("aria-label")));
  const previousPages = await pageNumbers();
  await page.locator(".appShell").focus();
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(before => JSON.stringify([...document.querySelectorAll(".pageSlot > .qulPage")].map(item => item.getAttribute("aria-label"))) !== JSON.stringify(before), previousPages);
  await page.waitForFunction(() => !document.querySelector(".bookFoldOverlay:not(.bookFoldWarmup)"));
  report.checks.push("Arrow navigation advances once and releases the page animation");

  await page.keyboard.press("Control+f");
  const search = page.locator(".primaryQuranSearch .searchInput");
  await search.waitFor();
  await search.fill("45");
  await search.press("ArrowLeft");
  assert.equal(await search.inputValue(), "45");
  await search.press("Enter");
  await page.waitForFunction(() => [...document.querySelectorAll(".pageSlot > .qulPage")].some(item => /\b45\b/.test(item.getAttribute("aria-label") ?? "")));
  report.checks.push("Ctrl+F, editable arrow handling and Enter page navigation remain functional");

  await page.locator(".pageSlot > .qulPage .ayahRun").first().click({ button: "right" });
  await page.getByRole("button", { name: /Tafsir/ }).click();
  await page.locator(".ayahStudyPanel").waitFor();
  const studyAyah = await page.locator(".studyNavigation select").inputValue();
  await page.locator(".appShell").focus();
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(previous => document.querySelector(".studyNavigation select").value !== previous, studyAyah);
  await page.getByRole("button", { name: "Close ayah study", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector(".studyOpen, .studyClosing"));
  report.checks.push("Right-click study menu and tafsir arrow navigation work in the native reader");

  await page.evaluate(() => window.chrome.webview.postMessage({ type: "fullscreen-request", active: true }));
  await page.waitForFunction(() => document.querySelector(".appShell").classList.contains("fullscreenReader"));
  assert.equal(await page.locator(".rustTitleBar").isVisible(), false);
  await page.locator(".fullscreenAccess").getByRole("button", { name: /Exit full screen/ }).click();
  await page.waitForFunction(() => !document.querySelector(".appShell").classList.contains("fullscreenReader"));
  await page.locator(".rustTitleBar").waitFor();
  await assertViewport("restored after fullscreen");
  report.checks.push("Native fullscreen removes chrome, provides a visible exit and restores the title bar");

  await revealOptions();
  await page.locator(".readerSettingsControl").click();
  await page.getByLabel("Theme", { exact: true }).selectOption("light");
  await page.waitForFunction(() => getComputedStyle(document.querySelector(".rustTitleBar")).backgroundColor === "rgb(241, 238, 232)");
  assert.equal(await page.locator(".rustTitleBar").evaluate(item => getComputedStyle(item).color), "rgb(21, 20, 18)");
  await page.getByLabel("Contextual letter elongation (optional)", { exact: true }).check();
  await page.getByRole("button", { name: "Close settings", exact: true }).click();
  await page.locator(".pageSlot > .qulPage .shapedLineInk").first().waitFor({ timeout: 30000 });
  report.checks.push("Light theme matches the title bar; contextual Quran rendering remains available");

  await revealOptions();
  await page.locator(".readerSettingsControl").click();
  await page.getByLabel("Theme", { exact: true }).selectOption("sepia");
  await page.waitForFunction(settingsKey => JSON.parse(localStorage.getItem(settingsKey)).theme === "sepia", settingsKey);
  await page.getByRole("button", { name: "Close settings", exact: true }).click();
  await page.evaluate(async ({ persistenceKey, cacheName }) => {
    localStorage.setItem(persistenceKey, "rust-profile-preserved");
    const cache = await caches.open(cacheName);
    await cache.put("/rust-host-qa-persistence", new Response("offline-data-preserved"));
  }, { persistenceKey, cacheName });
  await page.reload();
  await page.locator(".rustTitleBar").waitFor();
  await page.locator(".pageSlot > .qulPage .shapedLineInk").first().waitFor({ timeout: 30000 });
  await page.screenshot({ path: resolve(output, checkPersistence ? "after-restart.png" : "native-reader.png") });
  assert.deepEqual(report.consoleErrors, []);
  report.passed = true;
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  report.error = error instanceof Error ? { message: error.message, stack: error.stack } : String(error);
  throw error;
} finally {
  try { await browser?.close(); }
  catch (error) { report.disconnectError = String(error); }
  report.finishedAt = new Date().toISOString();
  await writeFile(reportPath, JSON.stringify(report, null, 2));
}
