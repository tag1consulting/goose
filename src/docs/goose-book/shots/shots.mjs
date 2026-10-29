#!/usr/bin/env node
// Retakes the Goose Book's screenshots from real load tests, as shots.json lists them.
// See README.md for what it needs and how to run it.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "../../../..");
const BOOK_SRC = path.resolve(HERE, "../src");
const WORK = path.join(HERE, ".work");
const MAX_BYTES = 500 * 1024;
// Text that means a panel has not filled yet.
const NOT_READY = /No data yet|No reports yet|connecting/;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (msg) => console.log(`[shots ${new Date().toISOString().slice(11, 19)}] ${msg}`);

function usage() {
  console.log(`usage: node shots.mjs [--list] [--only NAME[,NAME...]] [--skip-build]

  --list        list the shots and the runs they need, and exit
  --only        take only the named shots (a name is the shot's "name")
  --skip-build  use the binaries the last build left in .work/bin`);
}

function parseArgs(argv) {
  const opts = { list: false, only: null, skipBuild: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--list") opts.list = true;
    else if (a === "--skip-build") opts.skipBuild = true;
    else if (a === "--only") opts.only = new Set(argv[++i].split(","));
    else if (a.startsWith("--only=")) opts.only = new Set(a.slice(7).split(","));
    else if (a === "-h" || a === "--help") {
      usage();
      process.exit(0);
    } else {
      usage();
      throw new Error(`unknown argument ${a}`);
    }
  }
  return opts;
}

// Replaces ${VAR} with the environment variable, and fails on one that is not set.
function expand(value) {
  if (typeof value !== "string") return value;
  return value.replace(/\$\{([A-Z0-9_]+)\}/g, (_, name) => {
    const v = process.env[name];
    if (v === undefined || v === "") throw new Error(`environment variable ${name} is not set`);
    return v;
  });
}

function loadManifest() {
  const manifest = JSON.parse(fs.readFileSync(path.join(HERE, "shots.json"), "utf8"));
  const names = new Set();
  for (const shot of manifest.shots) {
    if (names.has(shot.name)) throw new Error(`two shots are named ${shot.name}`);
    names.add(shot.name);
    if (!manifest.runs[shot.run]) throw new Error(`shot ${shot.name} needs run ${shot.run}, which shots.json does not define`);
  }
  return manifest;
}

const featureKey = (run) => [...(run.features ?? [])].sort().join(",") || "default";

// Builds each example once per feature set, and copies each binary out so the next
// feature set's build does not overwrite it. A copy keeps its name, which Goose reports
// as the test's name.
function build(runs, skip) {
  const bin = path.join(WORK, "bin");
  fs.mkdirSync(bin, { recursive: true });
  const sets = new Map();
  for (const run of runs) {
    const key = featureKey(run);
    if (!sets.has(key)) sets.set(key, { features: run.features ?? [], examples: new Set() });
    sets.get(key).examples.add(run.example);
  }
  for (const [key, set] of sets) {
    const args = ["build", "--release"];
    for (const ex of set.examples) args.push("--example", ex);
    if (set.features.length) args.push("--features", set.features.join(","));
    if (!skip) {
      log(`cargo ${args.join(" ")}`);
      const r = spawnSync("cargo", args, { cwd: REPO, stdio: "inherit" });
      if (r.status !== 0) throw new Error(`cargo build failed for features ${key}`);
      fs.mkdirSync(path.join(bin, key), { recursive: true });
      for (const ex of set.examples) {
        fs.copyFileSync(path.join(REPO, "target/release/examples", ex), path.join(bin, key, ex));
        fs.chmodSync(path.join(bin, key, ex), 0o755);
      }
    }
  }
  return (run) => path.join(bin, featureKey(run), run.example);
}

const children = new Set();

function killAll() {
  for (const child of children) {
    try {
      child.kill("SIGKILL");
    } catch {}
  }
}
process.on("exit", killAll);
for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    killAll();
    process.exit(130);
  });
}

function startGoose(binary, args, env, cwd, logName) {
  const out = fs.openSync(path.join(cwd, logName), "w");
  const child = spawn(binary, args, { cwd, env: { ...process.env, ...env }, stdio: ["ignore", out, out] });
  children.add(child);
  child.exited = new Promise((resolve) => child.on("exit", (code, signal) => {
    children.delete(child);
    fs.closeSync(out);
    resolve({ code, signal });
  }));
  return child;
}

// Ctrl-C, as a user would stop Goose; a kill if it is still up 20 seconds later.
async function stopGoose(child) {
  if (!children.has(child)) return;
  child.kill("SIGINT");
  const done = await Promise.race([child.exited, sleep(20000).then(() => null)]);
  if (done === null) {
    child.kill("SIGKILL");
    await child.exited;
  }
}

async function api(base, token, method, route, body) {
  const headers = {};
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const res = await fetch(base + route, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  return { status: res.status, text };
}

async function waitForHealth(base, child, seconds) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) {
    if (!children.has(child)) throw new Error("Goose exited before its dashboard answered");
    try {
      if ((await fetch(base + "/api/v1/health")).ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error(`no dashboard at ${base} after ${seconds} seconds`);
}

async function portAnswers(base) {
  try {
    await fetch(base + "/api/v1/health", { signal: AbortSignal.timeout(1000) });
    return true;
  } catch {
    return false;
  }
}

async function doAction(page, ctx, action) {
  switch (action.do) {
    case "click":
      await page.locator(action.selector).click();
      break;
    case "fill":
      await page.locator(action.selector).fill(String(expand(action.value)));
      break;
    case "select":
      await page.locator(action.selector).selectOption(String(action.value));
      break;
    case "check":
      await page.locator(action.selector).check();
      break;
    case "open":
      // Opens a <details> element if it is closed.
      await page.locator(action.selector).evaluate((el) => {
        if (!el.open) el.querySelector("summary").click();
      });
      break;
    case "scroll":
      await page.locator(action.selector).scrollIntoViewIfNeeded();
      break;
    case "wait":
      await sleep(action.seconds * 1000);
      break;
    case "waitFor":
      await waitReady(page, action.selector, action.text, action.timeout ?? 120);
      break;
    case "api": {
      const r = await api(ctx.base, ctx.token, action.method ?? "POST", action.path, action.body);
      log(`  ${action.method ?? "POST"} ${action.path} -> ${r.status} ${r.text.slice(0, 160)}`);
      if (r.status >= 400) throw new Error(`${action.path} answered ${r.status}`);
      break;
    }
    case "telnet": {
      // One Controller command, as typed at the telnet prompt.
      const { connect } = await import("node:net");
      await new Promise((resolve, reject) => {
        const sock = connect(action.port ?? 5116, "127.0.0.1");
        let got = "";
        sock.on("data", (d) => {
          got += d;
          if (got.includes("goose> ") && !sock.sent) {
            sock.write(action.command + "\r\n");
            sock.sent = true;
            got = "";
          } else if (sock.sent && got.includes("\n")) {
            log(`  telnet ${action.command} -> ${got.trim().split("\n")[0]}`);
            sock.end("q\r\n");
            resolve();
          }
        });
        sock.on("error", reject);
      });
      break;
    }
    default:
      throw new Error(`unknown action ${JSON.stringify(action)}`);
  }
}

// Waits until the element is visible and holds data: none of NOT_READY, and `text` if given.
async function waitReady(page, selector, text, seconds) {
  await page.waitForFunction(
    ([sel, pattern, notReady]) => {
      const el = document.querySelector(sel);
      if (!el) return false;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) return false;
      const t = (el instanceof HTMLElement ? el.innerText : el.textContent) ?? "";
      if (new RegExp(notReady).test(t)) return false;
      return pattern ? new RegExp(pattern).test(t) : true;
    },
    [selector, text ?? null, NOT_READY.source],
    { timeout: seconds * 1000, polling: 500 },
  );
}

async function capture(page, ctx, shot, file) {
  const c = shot.capture;
  const pad = c.pad ?? 0;
  if (c.type === "element") {
    await waitReady(page, c.selector, c.text, 120);
    await page.locator(c.selector).scrollIntoViewIfNeeded();
    if (pad === 0 && !c.width && !c.height && !c.rows) return page.locator(c.selector).screenshot({ path: file, animations: "disabled" });
    return clipShot(page, [c.selector], pad, file, c);
  }
  if (c.type === "band") {
    for (const sel of [c.from, c.to]) await waitReady(page, sel, null, 120);
    return clipShot(page, [c.from, c.to], pad, file, c);
  }
  if (c.type === "viewport") {
    if (c.ready) await waitReady(page, c.ready, c.text, 120);
    await page.evaluate(() => window.scrollTo(0, 0));
    if (!c.column) return page.screenshot({ path: file, animations: "disabled" });
    // The first screen, cut to the left and right edges of the `column` element.
    const box = await page.locator(c.column).first().boundingBox();
    const size = page.viewportSize();
    const x = Math.max(0, box.x - pad);
    const width = Math.min(size.width - x, box.width + 2 * pad);
    return page.screenshot({ path: file, clip: { x, y: 0, width, height: size.height }, animations: "disabled" });
  }
  if (c.type === "page") {
    if (c.ready) await waitReady(page, c.ready, c.text, 120);
    return page.screenshot({ path: file, fullPage: true, animations: "disabled" });
  }
  if (c.type === "report") {
    const report = await page.context().newPage();
    try {
      await report.goto(pathToFileURL(findReport(ctx.cwd, c.file)).href);
      await sleep(2000);
      if (c.selector) return await clipShot(report, [c.selector].flat(), pad, file, c);
      return await report.screenshot({ path: file, fullPage: true, animations: "disabled" });
    } finally {
      await report.close();
    }
  }
  if (c.type === "pdf") {
    // The top of the PDF report, drawn by pdftoppm at `dpi`, cut to `height` pixels.
    const pdf = findReport(ctx.cwd, c.file);
    const prefix = file.replace(/\.png$/, "");
    const args = ["-png", "-singlefile", "-r", String(c.dpi ?? 150), "-f", "1", "-l", "1"];
    if (c.height) args.push("-x", "0", "-y", "0", "-W", String(c.width ?? 100000), "-H", String(c.height));
    const r = spawnSync("pdftoppm", [...args, pdf, prefix], { stdio: "inherit" });
    if (r.status !== 0) throw new Error(`pdftoppm failed on ${pdf}`);
    return;
  }
  throw new Error(`unknown capture type ${c.type}`);
}

// A report the run wrote: the newest run folder that holds `name`, or else `name` in the
// run's own folder, where a --report-file path without a folder writes it.
function findReport(cwd, name) {
  const runs = path.join(cwd, "goose-runs");
  const folders = fs.existsSync(runs) ? fs.readdirSync(runs).filter((d) => fs.existsSync(path.join(runs, d, name))).sort() : [];
  if (folders.length) return path.join(runs, folders.at(-1), name);
  if (fs.existsSync(path.join(cwd, name))) return path.join(cwd, name);
  throw new Error(`no ${name} in ${cwd}`);
}

// A screenshot of the box around every selector, plus `pad` pixels, in page coordinates,
// cut to at most `width` and `height` CSS pixels from its top left corner when given.
async function clipShot(page, selectors, pad, file, limits = {}) {
  const boxes = [];
  for (const sel of selectors) {
    await page.locator(sel).first().scrollIntoViewIfNeeded();
    const box = await page.locator(sel).first().evaluate((el) => {
      const r = el.getBoundingClientRect();
      return { x: r.left + window.scrollX, y: r.top + window.scrollY, right: r.right + window.scrollX, bottom: r.bottom + window.scrollY };
    });
    boxes.push(box);
  }
  const x = Math.max(0, Math.min(...boxes.map((b) => b.x)) - pad);
  const y = Math.max(0, Math.min(...boxes.map((b) => b.y)) - pad);
  let right = Math.max(...boxes.map((b) => b.right)) + pad;
  let bottom = Math.max(...boxes.map((b) => b.bottom)) + pad;
  if (limits.rows) {
    // Ends the clip under the given number of table rows, after the header row, in the last element.
    const rowBottom = await page.locator(selectors.at(-1)).first().evaluate((el, n) => {
      const rows = el.querySelectorAll("tr");
      const row = rows[Math.min(n, rows.length - 1)];
      return row.getBoundingClientRect().bottom + window.scrollY;
    }, limits.rows);
    bottom = Math.min(bottom, rowBottom + 1);
  }
  if (limits.width) right = Math.min(right, x + limits.width);
  if (limits.height) bottom = Math.min(bottom, y + limits.height);
  return page.screenshot({ path: file, fullPage: true, clip: { x, y, width: right - x, height: bottom - y }, animations: "disabled" });
}

function optimize(file) {
  const which = spawnSync("sh", ["-c", "command -v oxipng"], { encoding: "utf8" });
  if (which.status !== 0) return false;
  const r = spawnSync("oxipng", ["-o", "4", "--strip", "safe", "-q", file], { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`oxipng failed on ${file}`);
  return true;
}

class FailedRun extends Error {}

async function takeRun(browser, binary, name, run, shots, manifest, problems) {
  const cwd = path.join(WORK, "runs", name);
  fs.rmSync(cwd, { recursive: true, force: true });
  fs.mkdirSync(cwd, { recursive: true });
  const base = run.dashboard ?? "http://127.0.0.1:5118";
  const token = run.token ? expand(run.token) : null;
  const env = Object.fromEntries(Object.entries(run.env ?? {}).map(([k, v]) => [k, expand(v)]));
  const host = ["-H", expand(run.target)];

  if (await portAnswers(base)) throw new Error(`something already answers on ${base}; stop it first`);

  // A command to run first, such as one that reopens a tunnel to the target.
  if (process.env.GOOSE_SHOTS_BEFORE_RUN) {
    log(`run ${name}: ${process.env.GOOSE_SHOTS_BEFORE_RUN}`);
    const r = spawnSync("sh", ["-c", process.env.GOOSE_SHOTS_BEFORE_RUN], { stdio: "inherit" });
    if (r.status !== 0) throw new Error(`GOOSE_SHOTS_BEFORE_RUN exited ${r.status}`);
  }

  // A load target the rig starts for this run, such as a local server that stalls requests.
  const server = run.server ? await startServer(name, run.server, cwd) : null;
  try {
    await runGoose(browser, binary, name, run, shots, manifest, problems, { cwd, base, token, env, host });
  } finally {
    if (server) {
      server.kill("SIGTERM");
      await server.exited;
      log(`run ${name}: server stopped`);
    }
  }
}

async function startServer(name, spec, cwd) {
  const command = [expand(spec.command), ...(spec.args ?? []).map((a) => shellQuote(String(expand(a))))].join(" ");
  log(`run ${name}: server: ${command}`);
  const out = fs.openSync(path.join(cwd, "server.log"), "w");
  const child = spawn("sh", ["-c", `exec ${command}`], { cwd, stdio: ["ignore", out, out] });
  children.add(child);
  child.exited = new Promise((resolve) => child.on("exit", (code, signal) => {
    children.delete(child);
    fs.closeSync(out);
    resolve({ code, signal });
  }));
  const end = Date.now() + 30000;
  while (Date.now() < end) {
    if (!children.has(child)) throw new Error(`run ${name}: the server exited; see ${cwd}/server.log`);
    try {
      if ((await fetch(expand(spec.ready))).ok) return child;
    } catch {}
    await sleep(250);
  }
  child.kill("SIGKILL");
  throw new Error(`run ${name}: no answer from ${spec.ready} after 30 seconds`);
}

const shellQuote = (s) => (/^[\w@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, "'\\''")}'`);

async function runGoose(browser, binary, name, run, shots, manifest, problems, { cwd, base, token, env, host }) {
  for (const [i, flags] of (run.prelude ?? []).entries()) {
    log(`run ${name}: prelude ${i + 1}: ${flags.join(" ")}`);
    const child = startGoose(binary, [...host, ...flags.map(expand)], env, cwd, `prelude-${i + 1}.log`);
    const { code } = await child.exited;
    if (code !== 0) throw new Error(`run ${name}: prelude ${i + 1} exited ${code}; see ${cwd}`);
  }

  const args = [...host, ...run.flags.map(expand)];
  log(`run ${name}: ${run.example} ${args.join(" ")}`);
  const child = startGoose(binary, args, env, cwd, "goose.log");
  const started = Date.now();
  const context = await browser.newContext({
    viewport: manifest.defaults.viewport,
    deviceScaleFactor: manifest.defaults.scale,
    colorScheme: manifest.defaults.colorScheme,
    locale: manifest.defaults.locale,
    timezoneId: manifest.defaults.timezone,
  });
  try {
    await waitForHealth(base, child, 120);
    const page = await context.newPage();
    await page.goto(token ? `${base}/?token=${encodeURIComponent(token)}` : `${base}/`);
    const ctx = { base, token, cwd };
    // Shots, and the run's own events (actions with no capture), in time order.
    const events = (run.events ?? []).map((e, i) => ({ ...e, name: e.name ?? `event ${i + 1}`, event: true }));
    for (const shot of [...shots, ...events].sort((a, b) => a.at - b.at)) {
      const wait = shot.at * 1000 - (Date.now() - started);
      if (wait > 0) log(`run ${name}: ${shot.name} at ${shot.at} s, waiting ${Math.round(wait / 1000)} s`);
      while (Date.now() - started < shot.at * 1000) {
        if (!children.has(child)) throw new Error(`run ${name} ended before ${shot.name} at ${shot.at} s`);
        await sleep(250);
      }
      if (shot.event) {
        log(`run ${name}: ${shot.name}`);
        for (const action of shot.actions ?? []) await doAction(page, ctx, action);
        continue;
      }
      await page.setViewportSize(shot.viewport ?? manifest.defaults.viewport);
      for (const action of shot.actions ?? []) await doAction(page, ctx, action);
      if (shot.settle) await sleep(shot.settle * 1000);
      const file = path.join(BOOK_SRC, shot.output);
      await capture(page, ctx, shot, file);
      const optimized = optimize(file);
      const bytes = fs.statSync(file).size;
      log(`wrote ${shot.output} (${Math.round(bytes / 1024)} KB${optimized ? ", oxipng" : ""}, ${Math.round((Date.now() - started) / 1000)} s in)`);
      if (bytes > MAX_BYTES) problems.push(`${shot.output} is ${Math.round(bytes / 1024)} KB, over 500 KB`);
    }
    await page.close();
    // A run whose images must show no errors is taken again if any request failed.
    if (!run.allowFailures) {
      const snap = await api(base, token, "GET", "/api/v1/snapshot");
      const failures = snap.status === 200 ? JSON.parse(snap.text).aggregate.total_failures : null;
      if (failures !== 0) throw new FailedRun(`run ${name} had ${failures ?? "unknown"} failed requests`);
    }
  } finally {
    await context.close();
    await stopGoose(child);
    log(`run ${name}: stopped`);
  }
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const manifest = loadManifest();
  let shots = manifest.shots;
  if (opts.only) {
    for (const n of opts.only) if (!shots.some((s) => s.name === n)) throw new Error(`no shot named ${n}`);
    shots = shots.filter((s) => opts.only.has(s.name));
  }
  if (opts.list) {
    for (const s of shots) console.log(`${s.name.padEnd(28)} ${String(s.at).padStart(4)} s  run ${s.run.padEnd(12)} ${s.output}`);
    return;
  }
  const runNames = [...new Set(shots.map((s) => s.run))];
  const binaryFor = build(runNames.map((n) => manifest.runs[n]), opts.skipBuild);
  const t0 = Date.now();
  const problems = [];
  const browser = await chromium.launch();
  try {
    for (const name of runNames) {
      const run = manifest.runs[name];
      const attempts = run.attempts ?? 3;
      for (let attempt = 1; ; attempt++) {
        try {
          // Problems count only for the attempt whose images are kept.
          const found = [];
          await takeRun(browser, binaryFor(run), name, run, shots.filter((s) => s.run === name), manifest, found);
          problems.push(...found);
          break;
        } catch (err) {
          if (!(err instanceof FailedRun) || attempt >= attempts) throw err;
          log(`${err.message}; taking it again (attempt ${attempt + 1} of ${attempts})`);
        }
      }
    }
  } finally {
    await browser.close();
    killAll();
  }
  if (spawnSync("sh", ["-c", "command -v oxipng"]).status !== 0) log("oxipng is not installed; images were not optimized");
  log(`${shots.length} shots from ${runNames.length} runs in ${Math.round((Date.now() - t0) / 1000)} s`);
  if (problems.length) {
    for (const p of problems) console.error(`problem: ${p}`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err.stack ?? String(err));
  killAll();
  process.exit(1);
});
