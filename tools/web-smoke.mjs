#!/usr/bin/env node
// Smoke test for the browser app: serves a built `dist` folder, opens it in headless
// Chrome over the DevTools protocol, waits for the app to say it is running, and fails on
// any console error, uncaught exception or panic. Optionally saves a screenshot.
//
// No npm packages: Node 22+ (built-in WebSocket and fetch) and a Chrome or Chromium.
//
//   node tools/web-smoke.mjs --dist apps/chamfera-web/dist --out shot.png
//   node tools/web-smoke.mjs --dist apps/chamfera-web/dist --renderer webgl --out gl.png
//
// Options:
//   --dist DIR        the trunk output to serve (default apps/chamfera-web/dist)
//   --out FILE        save a PNG screenshot of the page
//   --renderer R      "webgpu" (default: whatever the app picks) or "webgl" (forces the
//                     WebGL 2 fallback with ?renderer=webgl)
//   --no-webgpu       start Chrome without WebGPU, so the app must fall back by itself
//   --size WxH        window size (default 1400x900)
//   --scale F         device pixel ratio (default 1)
//   --timeout MS      how long to wait for the app (default 30000)
//   --settle MS       extra wait after it runs, before the screenshot (default 1500)
//   --chrome PATH     the browser (default: $CHROME, then google-chrome, chromium, ...)
//   --steps FILE      a JSON list of steps to run once the app runs (see below)
//   --profile DIR     reuse this Chrome profile (local storage survives between runs);
//                     default: a fresh one, removed afterwards
//   --downloads DIR   where downloads go (default: a temporary folder)
//
// Steps drive the page like a user, with trusted input events:
//   {"click": [x, y]}                        left click at CSS pixels (also "button")
//   {"clickText": "Export STEP…"}            click a menu or menu item by its text, once
//                                            the app shows it (<body data-targets>)
//   {"key": "o", "modifiers": ["ctrl"]}      a key press (modifiers: ctrl, shift, alt, meta)
//   {"choose": ["path", ...]}                answer the next file picker with these files
//   {"wait": 500}                            milliseconds
//   {"expect": "js expression", "what": "description"}   must become truthy within 10 s
//   {"download": "suffix", "minBytes": 1}    a download whose name ends so must complete
//   {"reload": true}                         reload the page and wait for the app again
//   {"shot": "file.png"}                     a screenshot
// Paths in steps are relative to the steps file.

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, isAbsolute, join, resolve, normalize, sep } from "node:path";

function parseArgs(argv) {
  const o = {
    dist: "apps/chamfera-web/dist",
    out: null,
    renderer: null,
    noWebgpu: false,
    size: "1400x900",
    scale: 1,
    timeout: 30000,
    settle: 1500,
    chrome: process.env.CHROME || null,
    steps: null,
    profile: null,
    downloads: null,
  };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      i += 1;
      if (i >= argv.length) throw new Error(`${a} needs a value`);
      return argv[i];
    };
    switch (a) {
      case "--dist": o.dist = next(); break;
      case "--out": o.out = next(); break;
      case "--renderer": o.renderer = next(); break;
      case "--no-webgpu": o.noWebgpu = true; break;
      case "--size": o.size = next(); break;
      case "--scale": o.scale = Number(next()); break;
      case "--timeout": o.timeout = Number(next()); break;
      case "--settle": o.settle = Number(next()); break;
      case "--chrome": o.chrome = next(); break;
      case "--steps": o.steps = next(); break;
      case "--profile": o.profile = next(); break;
      case "--downloads": o.downloads = next(); break;
      default: throw new Error(`unknown option ${a}`);
    }
  }
  return o;
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".json": "application/json",
};

function serve(dir) {
  const root = resolve(dir);
  const server = createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    if (url.pathname === "/favicon.ico") {
      // The page names its own icon; Chrome asks for this one anyway.
      res.writeHead(204).end();
      return;
    }
    if (url.pathname === "/__blank") {
      res.writeHead(200, { "content-type": "text/html" }).end("<!doctype html><title>blank</title>");
      return;
    }
    let path = normalize(join(root, decodeURIComponent(url.pathname)));
    if (!path.startsWith(root)) {
      res.writeHead(403).end();
      return;
    }
    if (existsSync(path) && statSync(path).isDirectory()) path = join(path, "index.html");
    if (!existsSync(path)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, {
      "content-type": MIME[extname(path)] || "application/octet-stream",
      "cache-control": "no-store",
    });
    res.end(readFileSync(path));
  });
  return new Promise((ok) => server.listen(0, "127.0.0.1", () => ok(server)));
}

function findChrome(given) {
  const candidates = given
    ? [given]
    : ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome"];
  for (const c of candidates) {
    if (c.includes(sep) && existsSync(c)) return c;
    for (const p of (process.env.PATH || "").split(":")) {
      if (p && existsSync(join(p, c))) return join(p, c);
    }
  }
  throw new Error("no Chrome or Chromium found (set --chrome or $CHROME)");
}

const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

async function waitFor(fn, timeout, what) {
  const end = Date.now() + timeout;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.pending = new Map();
    this.handlers = [];
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && this.pending.has(msg.id)) {
        const { ok, fail } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) fail(new Error(`${msg.error.message}`));
        else ok(msg.result);
      } else if (msg.method) {
        for (const h of this.handlers) h(msg);
      }
    });
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((ok, fail) => {
      ws.addEventListener("open", ok, { once: true });
      ws.addEventListener("error", () => fail(new Error("DevTools connection failed")), { once: true });
    });
    return new Cdp(ws);
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((ok, fail) => this.pending.set(id, { ok, fail }));
  }
  on(fn) {
    this.handlers.push(fn);
  }
  async eval(expression) {
    const r = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (r.exceptionDetails) throw new Error(`page script failed: ${r.exceptionDetails.text} ${r.exceptionDetails.exception?.description ?? ""}`);
    return r.result.value;
  }
}

async function main() {
  const o = parseArgs(process.argv);
  if (!existsSync(join(o.dist, "index.html"))) {
    throw new Error(`${o.dist}/index.html not found: build with trunk first`);
  }
  const [w, h] = o.size.split("x").map(Number);
  const server = await serve(o.dist);
  const port = server.address().port;
  const fresh = !o.profile;
  const profile = o.profile || mkdtempSync(join(tmpdir(), "chamfera-smoke-"));
  const flags = [
    "--headless=new",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    `--window-size=${w},${h}`,
    `--force-device-scale-factor=${o.scale}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--hide-scrollbars",
    "--mute-audio",
    // Headless machines (CI) have no GPU: WebGPU and WebGL 2 then run on SwiftShader.
    "--enable-unsafe-swiftshader",
  ];
  if (o.noWebgpu) {
    flags.push("--use-angle=swiftshader", "--disable-features=WebGPU,Vulkan", "--disable-webgpu");
  } else {
    // ANGLE and the compositor on SwiftShader's Vulkan too: with ANGLE on its GL path, a
    // WebGPU canvas draws but never reaches the screenshot.
    flags.push(
      "--use-angle=vulkan",
      "--use-vulkan=swiftshader",
      "--enable-features=Vulkan",
      "--enable-unsafe-webgpu",
      "--use-webgpu-adapter=swiftshader",
    );
  }
  // For trying other GPU setups: extra Chrome flags, separated by spaces.
  if (process.env.SMOKE_CHROME_FLAGS) flags.push(...process.env.SMOKE_CHROME_FLAGS.split(" ").filter(Boolean));
  // As root, and on CI runners (where user namespaces may be restricted), Chrome's sandbox
  // cannot start; the page is our own build.
  if ((process.getuid && process.getuid() === 0) || process.env.CI) flags.push("--no-sandbox");
  const chrome = spawn(findChrome(o.chrome), [...flags, "about:blank"], {
    stdio: ["ignore", "ignore", "pipe"],
  });
  let chromeLog = "";
  chrome.stderr.on("data", (d) => (chromeLog += d.toString()));
  const problems = [];
  let exitCode = 1;
  try {
    const portFile = join(profile, "DevToolsActivePort");
    const devPort = await waitFor(
      () => existsSync(portFile) && readFileSync(portFile, "utf8").split("\n")[0],
      // A fresh profile on a busy CI runner can take well over 15 s before the port opens.
      60000,
      "Chrome to start",
    );
    const targets = await waitFor(async () => {
      const list = await (await fetch(`http://127.0.0.1:${devPort}/json/list`)).json();
      return list.find((t) => t.type === "page");
    }, 15000, "a page target");
    const cdp = await Cdp.connect(targets.webSocketDebuggerUrl);
    const log = [];
    const downloads = [];
    let chooser = null;
    cdp.on((m) => {
      if (m.method === "Page.fileChooserOpened") chooser = m.params;
      if (m.method === "Browser.downloadWillBegin") {
        downloads.push({ guid: m.params.guid, name: m.params.suggestedFilename, state: "started" });
      }
      if (m.method === "Browser.downloadProgress") {
        const d = downloads.find((x) => x.guid === m.params.guid);
        if (d) {
          d.state = m.params.state;
          d.bytes = m.params.receivedBytes;
        }
      }
      if (m.method === "Runtime.consoleAPICalled") {
        const text = m.params.args.map((a) => a.value ?? a.description ?? "").join(" ");
        log.push(`[console.${m.params.type}] ${text}`);
        if (m.params.type === "error" || m.params.type === "assert") problems.push(`console error: ${text}`);
      } else if (m.method === "Runtime.exceptionThrown") {
        const d = m.params.exceptionDetails;
        problems.push(`exception: ${d.exception?.description ?? d.text}`);
      } else if (m.method === "Log.entryAdded") {
        const e = m.params.entry;
        log.push(`[${e.source}.${e.level}] ${e.text}`);
        // Chrome's own warnings about GPU fallbacks are not the app's errors.
        if (e.level === "error" && e.source !== "violation" && e.source !== "rendering") {
          problems.push(`browser error: ${e.text} ${e.url ?? ""}`);
        }
      }
    });
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Page.enable");
    const downloadDir = o.downloads ? resolve(o.downloads) : join(profile, "downloads");
    await cdp.send("Browser.setDownloadBehavior", {
      behavior: "allowAndName",
      downloadPath: downloadDir,
      eventsEnabled: true,
    });
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: w,
      height: h,
      deviceScaleFactor: o.scale,
      mobile: false,
    });
    if (!o.noWebgpu) {
      // Headless Chrome answers the first WebGPU adapter request with null while its GPU
      // process starts; ask on a blank page of the same origin until it answers.
      await cdp.send("Page.navigate", { url: `http://127.0.0.1:${port}/__blank` });
      const end = Date.now() + 15000;
      let adapter = null;
      while (Date.now() < end) {
        adapter = await cdp
          .eval(`navigator.gpu ? navigator.gpu.requestAdapter().then((a) => !!a) : false`)
          .catch(() => null);
        if (adapter) break;
        await sleep(250);
      }
      console.log(`WebGPU adapter before start: ${adapter ? "yes" : "no"}`);
    }
    const query = o.renderer ? `?renderer=${encodeURIComponent(o.renderer)}` : "";
    const url = `http://127.0.0.1:${port}/${query}`;
    await cdp.send("Page.navigate", { url });
    const started = () =>
      waitFor(
        () =>
          cdp.eval(`(() => {
            const b = document.body;
            const s = b && b.dataset.chamfera;
            return s === "running" || s === "failed" ? s : null;
          })()`).catch(() => null),
        o.timeout,
        "the app to start (body[data-chamfera])",
      );
    const state = await started();
    await sleep(o.settle);
    const info = await cdp.eval(`(() => {
      const c = document.querySelector("canvas#chamfera");
      return {
        state: document.body.dataset.chamfera,
        backend: document.body.dataset.backend || "",
        error: document.body.dataset.error || "",
        canvas: c ? { css: [c.clientWidth, c.clientHeight], px: [c.width, c.height] } : null,
        dpr: window.devicePixelRatio,
        webgpu: "gpu" in navigator,
      };
    })()`);
    console.log(`page: ${url}`);
    console.log(`state: ${info.state}, backend: ${info.backend}, navigator.gpu: ${info.webgpu}, dpr: ${info.dpr}`);
    if (info.canvas) console.log(`canvas: ${info.canvas.css.join("x")} css, ${info.canvas.px.join("x")} px`);
    if (state !== "running") problems.push(`the app did not start: ${info.error}`);
    if (!info.canvas) problems.push("no canvas#chamfera");
    else if (info.canvas.px[0] < 100 || info.canvas.px[1] < 100) problems.push("the canvas is not sized");
    if (o.renderer === "webgl" && !/gl/i.test(info.backend)) problems.push(`asked for WebGL, got ${info.backend}`);
    if (o.steps) {
      const base = dirname(resolve(o.steps));
      const at = (p) => (isAbsolute(p) ? p : join(base, p));
      const steps = JSON.parse(readFileSync(o.steps, "utf8"));
      const mods = (list = []) =>
        list.reduce((m, k) => m | ({ alt: 1, ctrl: 2, meta: 4, shift: 8 }[k] ?? 0), 0);
      let seen = 0;
      for (const [i, step] of steps.entries()) {
        const label = `step ${i + 1} ${JSON.stringify(step)}`;
        console.log(label);
        if (step.click || step.clickText) {
          // By text: the app lists the menus and menu items it drew on <body data-targets>
          // (text -> centre); wait until the one asked for is on screen.
          const [x, y] = step.click
            ? step.click
            : await waitFor(
                () =>
                  cdp
                    .eval(`(JSON.parse(document.body.dataset.targets || "{}"))[${JSON.stringify(step.clickText)}] || null`)
                    .catch(() => null),
                step.timeout || 30000,
                `"${step.clickText}" on screen`,
              );
          const button = step.button || "left";
          await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
          await sleep(100);
          await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button, clickCount: 1 });
          await sleep(50);
          await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button, clickCount: 1 });
          await sleep(300);
        } else if (step.key) {
          const key = step.key;
          const code = key.length === 1 ? `Key${key.toUpperCase()}` : key;
          const keyCode = key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0;
          const m = mods(step.modifiers);
          for (const type of ["rawKeyDown", "keyUp"]) {
            await cdp.send("Input.dispatchKeyEvent", {
              type, key, code, modifiers: m, windowsVirtualKeyCode: keyCode,
            });
            await sleep(50);
          }
          await sleep(300);
        } else if (step.choose) {
          chooser = null;
          await cdp.send("Page.setInterceptFileChooserDialog", { enabled: true });
          step.pending = step.choose.map(at);
          o.pendingFiles = step.pending;
        } else if (step.wait) {
          await sleep(step.wait);
        } else if (step.expect) {
          await waitFor(
            () => cdp.eval(step.expect).catch(() => false),
            step.timeout || 30000,
            step.what || step.expect,
          );
        } else if (step.download) {
          const d = await waitFor(
            () => downloads.slice(seen).find((x) => x.name.endsWith(step.download) && x.state !== "started"),
            step.timeout || 30000,
            `a download ending in ${step.download}`,
          );
          seen = downloads.indexOf(d) + 1;
          if (d.state !== "completed") throw new Error(`download ${d.name}: ${d.state}`);
          if ((d.bytes ?? 0) < (step.minBytes ?? 1)) throw new Error(`download ${d.name} has ${d.bytes} bytes`);
          console.log(`  downloaded ${d.name} (${d.bytes} bytes) to ${join(downloadDir, d.guid)}`);
        } else if (step.reload) {
          await cdp.eval(`delete document.body.dataset.chamfera; true`).catch(() => {});
          await cdp.send("Page.reload", {});
          if ((await started()) !== "running") throw new Error("the app did not start again");
          await sleep(o.settle);
        } else if (step.shot) {
          const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
          writeFileSync(at(step.shot), Buffer.from(shot.data, "base64"));
          console.log(`  screenshot: ${at(step.shot)}`);
        } else {
          throw new Error(`unknown ${label}`);
        }
        // A picker the page opened is answered with the files from the last "choose".
        if (o.pendingFiles && (step.click || step.clickText || step.key)) {
          const end = Date.now() + 3000;
          while (!chooser && Date.now() < end) await sleep(100);
        }
        if (chooser && o.pendingFiles) {
          await cdp.send("DOM.setFileInputFiles", {
            files: o.pendingFiles,
            backendNodeId: chooser.backendNodeId,
          });
          console.log(`  picked ${o.pendingFiles.map((f) => basename(f)).join(", ")}`);
          chooser = null;
          o.pendingFiles = null;
        }
      }
      if (o.pendingFiles) problems.push("a file was offered but the page never opened a picker");
    }
    if (o.out) {
      const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
      writeFileSync(o.out, Buffer.from(shot.data, "base64"));
      console.log(`screenshot: ${o.out}`);
    }
    for (const l of log) console.log(l);
    await cdp.send("Browser.close").catch(() => {});
    exitCode = problems.length ? 1 : 0;
  } catch (e) {
    problems.push(String(e && e.message ? e.message : e));
    if (chromeLog) console.error(chromeLog.split("\n").slice(-20).join("\n"));
  } finally {
    chrome.kill();
    server.close();
    if (fresh) {
      await sleep(300);
      rmSync(profile, { recursive: true, force: true });
    }
  }
  for (const p of problems) console.error(`FAIL: ${p}`);
  console.log(exitCode === 0 ? "web smoke test: ok" : "web smoke test: FAILED");
  process.exit(exitCode);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
