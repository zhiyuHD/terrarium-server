// server/release-runner.ts
import { realpathSync } from "node:fs";
import { spawn } from "node:child_process";
import { join as join2 } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

// server/releases.ts
import { chmod, copyFile, lstat, mkdir, open, readFile, readdir, rename, rm, symlink } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createHash, randomUUID } from "node:crypto";
var idPattern = /^[a-z0-9][a-z0-9._-]{0,79}$/;
var commonRequired = ["admin.mjs", "release.mjs", "runner.mjs", "index.html", "scene.js", "console/index.html", "console/client.js", "console/client.css", "console/assets/icons.svg", "console/assets/MiSans-Regular.woff2", "console/assets/MiSans-Semibold.woff2", "console/assets/MiSans-LICENSE.pdf", "console/assets/Lucide-LICENSE.txt"];
var nativeExecutable = "server";
function requiredFiles(runtime) {
  return [runtime ? nativeExecutable : "server.mjs", ...commonRequired];
}
function validReleaseId(id) {
  return typeof id === "string" && idPattern.test(id) && id !== "." && id !== "..";
}
function validPath(path) {
  return typeof path === "string" && ([nativeExecutable, "server.mjs", ...commonRequired].includes(path) || /^console\/assets\/[a-zA-Z0-9][a-zA-Z0-9._-]{0,99}$/.test(path));
}
function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
async function syncDirectory(path) {
  const handle = await open(path, "r");
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}
async function durableJson(path, value) {
  await mkdir(dirname(path), { recursive: true, mode: 448 });
  const temp = `${path}.${randomUUID()}.tmp`, handle = await open(temp, "wx", 384);
  try {
    await handle.writeFile(JSON.stringify(value, null, 2) + "\n");
    await handle.sync();
  } finally {
    await handle.close();
  }
  try {
    await rename(temp, path);
    await syncDirectory(dirname(path));
  } finally {
    await rm(temp, { force: true });
  }
}
async function filesIn(dir, prefix = "") {
  const result = [];
  for (const item of await readdir(dir)) {
    const relative = prefix + item, info = await lstat(join(dir, item));
    if (info.isDirectory()) result.push(...await filesIn(join(dir, item), `${relative}/`));
    else if (info.isFile()) result.push(relative);
    else throw new Error(`Release contains a non-regular file: ${relative}`);
  }
  return result.sort();
}
async function fileBytes(dir, path) {
  const info = await lstat(join(dir, path));
  if (!info.isFile() || info.size > (path === nativeExecutable ? 128 : 32) * 1024 * 1024) throw new Error(`Invalid release file: ${path}`);
  return readFile(join(dir, path));
}
async function verifyRelease(dir) {
  if (!(await lstat(dir)).isDirectory()) throw new Error("Release must be a directory");
  const raw = await fileBytes(dir, "release.json");
  if (raw.length > 65536) throw new Error("Release manifest is too large");
  const manifest = JSON.parse(raw.toString());
  if (![1, 2].includes(manifest.schema) || (manifest.schema === 1 ? manifest.runtime !== void 0 : !manifest.runtime || manifest.runtime.kind !== "native" || manifest.runtime.platform !== "linux" || !["x64", "arm64"].includes(manifest.runtime.arch)) || !validReleaseId(manifest.id) || typeof manifest.version !== "string" || manifest.version.length > 80 || typeof manifest.revision !== "string" || manifest.revision.length > 80 || !Number.isFinite(Date.parse(manifest.createdAt)) || !Number.isSafeInteger(manifest.dataEpoch) || manifest.dataEpoch < 1 || ![0, 1].includes(manifest.healthProtocol) || !Array.isArray(manifest.files) || manifest.files.length > 128 || !manifest.files.every((file) => file && validPath(file.path) && Number.isSafeInteger(file.size) && file.size >= 0 && /^[a-f0-9]{64}$/.test(file.sha256))) throw new Error("Invalid release manifest");
  const names = manifest.files.map((file) => file.path);
  if (new Set(names).size !== names.length || requiredFiles(manifest.runtime).some((path) => !names.includes(path)) || (manifest.runtime ? names.includes("server.mjs") : names.includes(nativeExecutable)) || manifest.files.reduce((total, file) => total + file.size, 0) > 192 * 1024 * 1024) throw new Error("Incomplete or oversized release");
  const actual = await filesIn(dir);
  if (JSON.stringify(actual) !== JSON.stringify(["release.json", ...names].sort())) throw new Error("Release file inventory mismatch");
  for (const file of manifest.files) {
    const bytes = await fileBytes(dir, file.path);
    if (bytes.length !== file.size || sha256(bytes) !== file.sha256) throw new Error(`Release checksum mismatch: ${file.path}`);
  }
  return manifest;
}
async function copyPayload(source, target, paths) {
  await mkdir(target, { recursive: true });
  for (const path of paths) {
    await mkdir(dirname(join(target, path)), { recursive: true });
    await copyFile(join(source, path), join(target, path));
  }
}
async function seal(dir, runtime) {
  for (const item of await readdir(dir)) {
    const path = join(dir, item);
    if ((await lstat(path)).isDirectory()) await seal(path);
    else {
      await chmod(path, runtime && item === nativeExecutable ? 365 : 292);
      const file = await open(path, "r");
      try {
        await file.sync();
      } finally {
        await file.close();
      }
    }
  }
  await chmod(dir, 365);
  await syncDirectory(dir);
}
async function removeStaging(dir) {
  try {
    await chmod(dir, 448);
  } catch (error) {
    if (error.code === "ENOENT") return;
    throw error;
  }
  for (const item of await readdir(dir)) {
    const path = join(dir, item);
    if ((await lstat(path)).isDirectory()) await removeStaging(path);
  }
  await rm(dir, { recursive: true, force: true });
}
var ReleaseStore = class {
  root;
  constructor(root) {
    this.root = resolve(root);
  }
  get stateFile() {
    return join(this.root, ".deployment/state.json");
  }
  directory(id) {
    if (!validReleaseId(id)) throw new Error("Invalid release ID");
    return join(this.root, "versions", id);
  }
  async state() {
    const state = JSON.parse(await readFile(this.stateFile, "utf8"));
    const pending = state.pending;
    if (state.schema !== 1 || !validReleaseId(state.controller) || !validReleaseId(state.current) || state.previous !== null && !validReleaseId(state.previous) || !Array.isArray(state.history) || state.history.length > 100 || !state.history.every((item) => item && typeof item.id === "string" && ["activate", "rollback"].includes(item.action) && validReleaseId(item.from) && validReleaseId(item.to) && ["succeeded", "failed"].includes(item.status) && Number.isFinite(Date.parse(item.finishedAt)) && (item.error === void 0 || typeof item.error === "string")) || pending !== null && (!pending || pending.from !== state.current || !validReleaseId(pending.to) || pending.from === pending.to || !["queued", "starting"].includes(pending.phase) || !["activate", "rollback"].includes(pending.action) || typeof pending.id !== "string" || !Number.isFinite(Date.parse(pending.requestedAt)))) throw new Error("Invalid deployment state");
    return state;
  }
  async manifest(id) {
    const manifest = await verifyRelease(this.directory(id));
    if (manifest.id !== id) throw new Error("Release directory ID mismatch");
    return manifest;
  }
  async save(state) {
    await durableJson(this.stateFile, state);
  }
  async install(source) {
    const manifest = await verifyRelease(source), destination = this.directory(manifest.id);
    await mkdir(join(this.root, "versions"), { recursive: true });
    try {
      const existing = await verifyRelease(destination);
      if (JSON.stringify(existing) !== JSON.stringify(manifest)) throw new Error("Release ID already contains different files");
      return existing;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    const temp = join(this.root, "versions", `.install-${randomUUID()}`);
    try {
      await copyPayload(source, temp, ["release.json", ...manifest.files.map((file) => file.path)]);
      if (JSON.stringify(await verifyRelease(temp)) !== JSON.stringify(manifest)) throw new Error("Release changed during installation");
      await seal(temp, manifest.runtime);
      await rename(temp, destination);
      await syncDirectory(join(this.root, "versions"));
    } finally {
      await removeStaging(temp);
    }
    return manifest;
  }
  async switchCurrent(id) {
    await this.manifest(id);
    const temp = join(this.root, `.current-${randomUUID()}`);
    try {
      await symlink(`versions/${id}`, temp);
      await rename(temp, join(this.root, "current"));
      await syncDirectory(this.root);
    } finally {
      await rm(temp, { force: true });
    }
  }
  async initialize(id) {
    await this.manifest(id);
    try {
      const state = await this.state();
      if (state.current !== id || state.controller !== id || state.pending || state.history.length) throw new Error("Deployment is already initialized");
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      await this.save({ schema: 1, controller: id, current: id, previous: null, pending: null, history: [] });
    }
    await this.switchCurrent(id);
    const temp = join(this.root, `.manager-${randomUUID()}`);
    try {
      await symlink(`versions/${id}`, temp);
      await rename(temp, join(this.root, "manager"));
      await syncDirectory(this.root);
    } finally {
      await rm(temp, { force: true });
    }
  }
  async upgradeController(id) {
    const state = await this.state();
    if (state.pending) throw new Error("A deployment transaction is pending");
    await this.manifest(id);
    const temp = join(this.root, `.manager-${randomUUID()}`);
    try {
      await symlink(`versions/${id}`, temp);
      await rename(temp, join(this.root, "manager"));
      await syncDirectory(this.root);
    } finally {
      await rm(temp, { force: true });
    }
    await this.save({ ...state, controller: id });
  }
  async queue(target, action) {
    const state = await this.state();
    if (state.pending) throw new Error("A deployment transaction is pending; restart the runner to recover it");
    const to = target ?? (action === "rollback" ? state.previous : null);
    if (!to || to === state.current) throw new Error("Choose a release different from current");
    const current = await this.manifest(state.current), next = await this.manifest(to);
    if (next.dataEpoch !== current.dataEpoch) throw new Error("Release data compatibility differs; an explicit data migration is required");
    const pending = { id: randomUUID(), action, from: state.current, to, phase: "queued", requestedAt: (/* @__PURE__ */ new Date()).toISOString() };
    await this.save({ ...state, pending });
    return pending;
  }
  async starting(transaction) {
    const state = await this.state();
    if (state.pending?.id !== transaction.id) throw new Error("Deployment transaction changed");
    await this.save({ ...state, pending: { ...transaction, phase: "starting" } });
  }
  async finish(transaction, error) {
    const state = await this.state();
    if (state.pending?.id !== transaction.id) throw new Error("Deployment transaction changed");
    const result = { id: transaction.id, action: transaction.action, from: transaction.from, to: transaction.to, status: error === void 0 ? "succeeded" : "failed", finishedAt: (/* @__PURE__ */ new Date()).toISOString(), ...error === void 0 ? {} : { error: error.slice(0, 1e3) } };
    await this.save({ schema: 1, controller: state.controller, current: error === void 0 ? transaction.to : transaction.from, previous: error === void 0 ? transaction.from : state.previous, pending: null, history: [...state.history, result].slice(-100) });
    return result;
  }
  async list() {
    const state = await this.state(), releases = [];
    for (const id of (await readdir(join(this.root, "versions"))).filter(validReleaseId).sort()) {
      try {
        releases.push({ id, current: state.current === id, previous: state.previous === id, manifest: await this.manifest(id) });
      } catch (error) {
        releases.push({ id, current: state.current === id, previous: state.previous === id, error: error.message });
      }
    }
    return { ...state, releases };
  }
};

// server/release-runner.ts
async function runDeployment(root) {
  const store = new ReleaseStore(root);
  const timeoutMs = Number(process.env.CORTICO_TERRARIUM_START_TIMEOUT_MS ?? 2e4);
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1500 || timeoutMs > 12e4) throw new Error("Invalid server startup timeout");
  const healthUrl = new URL(process.env.CORTICO_TERRARIUM_HEALTH_URL ?? `http://127.0.0.1:${process.env.PORT ?? 8787}/health`);
  if (healthUrl.protocol !== "http:" || healthUrl.hostname !== "127.0.0.1" || healthUrl.pathname !== "/health") throw new Error("Release health URL must use loopback HTTP /health");
  let active, stopping = false;
  const stop = async (server) => {
    if (!server || server.ended) return;
    server.child.kill("SIGTERM");
    const timeout = setTimeout(() => {
      server.child.kill("SIGKILL");
    }, 3e3);
    timeout.unref();
    try {
      await server.done;
    } finally {
      clearTimeout(timeout);
    }
  };
  const shutdown = () => {
    stopping = true;
    void stop(active);
  };
  process.on("SIGTERM", shutdown);
  process.on("SIGINT", shutdown);
  const launch = (manifest) => {
    const runtime = manifest.runtime;
    if (runtime && (runtime.platform !== process.platform || runtime.arch !== process.arch)) throw new Error("Native release target does not match this host");
    const executable = runtime ? join2(store.directory(manifest.id), "server") : process.execPath;
    const args = runtime ? [] : [join2(store.directory(manifest.id), "server.mjs")];
    const child = spawn(executable, args, { cwd: store.root, stdio: "inherit", env: { ...process.env, TERRARIUM_SERVER_MAIN: "1", CORTICO_TERRARIUM_RELEASE_ID: manifest.id, CORTICO_TERRARIUM_DEPLOY_ROOT: store.root } });
    const server = { child, done: Promise.resolve(), ended: false };
    server.done = new Promise((resolve2) => {
      child.once("error", (error) => {
        server.error = error.message;
        server.ended = true;
        resolve2();
      });
      child.once("exit", (code, signal) => {
        server.error = `Server exited (${signal ?? code})`;
        server.ended = true;
        resolve2();
      });
    });
    active = server;
    return server;
  };
  const healthy = async (server, manifest) => {
    const deadline = Date.now() + timeoutMs;
    let readySince = 0, lastError = "";
    while (Date.now() < deadline) {
      if (stopping) throw new Error("Runner stopped during activation");
      if (server.ended) throw new Error(server.error);
      try {
        const response = await fetch(healthUrl, { signal: AbortSignal.timeout(1e3), redirect: "error" });
        const health = await response.json();
        if (!response.ok || health.ok !== true || manifest.healthProtocol === 1 && health.releaseId !== manifest.id) throw new Error("Server release health check did not match");
        const urls = [new URL("/", healthUrl)];
        if (process.env.CORTICO_TERRARIUM_ADMIN_HOST) {
          const admin = `http://${process.env.CORTICO_TERRARIUM_ADMIN_HOST}:${process.env.CORTICO_TERRARIUM_ADMIN_PORT ?? 8788}`;
          urls.push(new URL("/", admin), new URL("/client.js", admin), new URL("/client.css", admin));
        }
        for (const url of urls) {
          const result = await fetch(url, { signal: AbortSignal.timeout(1e3), redirect: "error" });
          await result.body?.cancel();
          if (!result.ok) throw new Error(`Server asset check failed: ${url.pathname} (${result.status})`);
        }
        if (!readySince) readySince = Date.now();
        if (Date.now() - readySince >= 750 && !server.ended) return;
      } catch (error) {
        lastError = error.message;
        readySince = 0;
      }
      await delay(150);
    }
    throw new Error(`Server startup timed out: ${lastError}`);
  };
  try {
    let state = await store.state();
    if (state.pending?.phase === "starting") {
      const pending = state.pending;
      await store.switchCurrent(pending.from);
      await store.finish(pending, "Runner interrupted before activation was committed");
      console.error(`Recovered interrupted deployment ${pending.to}; restored ${pending.from}`);
      state = await store.state();
    }
    if (state.pending) {
      const pending = state.pending;
      try {
        const manifest = await store.manifest(pending.to);
        const previous = await store.manifest(pending.from);
        if (manifest.dataEpoch !== previous.dataEpoch) throw new Error("Release data compatibility differs");
        await store.starting(pending);
        await store.switchCurrent(pending.to);
        const candidate = launch(manifest);
        await healthy(candidate, manifest);
        if (stopping) return;
        await store.finish(pending);
        console.log(`Activated ${pending.to}; previous ${pending.from}`);
      } catch (error) {
        await stop(active);
        active = void 0;
        if (stopping) return;
        await store.switchCurrent(pending.from);
        await store.finish(pending, error.message);
        console.error(`Activation failed; restored ${pending.from}: ${error.message}`);
      }
    }
    if (!active && !stopping) {
      state = await store.state();
      const manifest = await store.manifest(state.current);
      await store.switchCurrent(state.current);
      const server = launch(manifest);
      await healthy(server, manifest);
      console.log(`Serving ${manifest.id}`);
    }
    if (active) {
      await active.done;
      if (!stopping) throw new Error(active.error);
    }
  } finally {
    await stop(active);
    process.off("SIGTERM", shutdown);
    process.off("SIGINT", shutdown);
  }
}
if (process.argv[1] && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  const root = process.env.CORTICO_TERRARIUM_DEPLOY_ROOT;
  if (!root) {
    console.error("CORTICO_TERRARIUM_DEPLOY_ROOT is required");
    process.exitCode = 1;
  } else void runDeployment(root).catch((error) => {
    console.error(error.stack);
    process.exitCode = 1;
  });
}
export {
  runDeployment
};
