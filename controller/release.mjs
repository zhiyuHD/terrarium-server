// server/release-cli.ts
import { realpathSync } from "node:fs";
import { mkdir as mkdir2, open as open2 } from "node:fs/promises";
import { spawn } from "node:child_process";
import { dirname as dirname2, join as join2, resolve as resolve2 } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { setTimeout as delay } from "node:timers/promises";

// server/releases.ts
import { chmod, copyFile, lstat, mkdir, open, readFile, readdir, rename, rm, symlink } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createHash, randomUUID } from "node:crypto";
var RELEASE_DATA_EPOCH = 1;
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
async function packRelease(source, output, metadata) {
  if (!validReleaseId(metadata.id)) throw new Error("Invalid release ID");
  const paths = [.../* @__PURE__ */ new Set([...requiredFiles(metadata.runtime), ...(await filesIn(join(source, "console/assets"))).map((path) => "console/assets/" + path)])];
  if (paths.some((path) => !validPath(path))) throw new Error("Unsupported release payload path");
  const temp = resolve(dirname(output), `.pack-${randomUUID()}`);
  await mkdir(dirname(output), { recursive: true });
  try {
    for (const path of paths) {
      if (!(await lstat(join(source, path))).isFile()) throw new Error(`Invalid source file: ${path}`);
    }
    await copyPayload(source, temp, paths);
    const files = await Promise.all(paths.sort().map(async (path) => {
      const bytes = await fileBytes(temp, path);
      return { path, size: bytes.length, sha256: sha256(bytes) };
    }));
    const manifest = { schema: metadata.runtime ? 2 : 1, ...metadata, createdAt: (/* @__PURE__ */ new Date()).toISOString(), dataEpoch: metadata.dataEpoch ?? RELEASE_DATA_EPOCH, healthProtocol: metadata.healthProtocol ?? 1, files };
    await durableJson(join(temp, "release.json"), manifest);
    await verifyRelease(temp);
    await seal(temp, metadata.runtime);
    await rename(temp, output);
    await syncDirectory(dirname(resolve(output)));
    return manifest;
  } finally {
    await removeStaging(temp);
  }
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

// server/release-cli.ts
var help = `pack --source <build-dir> --output <new-dir> --id <release-id> --version <version> --revision <commit> [--legacy-health] [--native-target linux-x64|linux-arm64 --data-epoch <number>]
install --source <release-dir> --root <deployment>
init --id <installed-release> --root <deployment>
upgrade-controller --id <installed-release> --root <deployment>
status | list | verify --id <release-id>
activate --id <release-id> [--service cortico-box.service]
rollback [--id <release-id>] [--service cortico-box.service]
recover [--service cortico-box.service]
Deployment commands use --root or CORTICO_TERRARIUM_DEPLOY_ROOT. Updates and rollback restart the user systemd service; data/ is never copied or restored.`;
async function command(program, args, env = process.env) {
  const child = spawn(program, args, { stdio: "inherit", env });
  return new Promise((resolve3, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => signal ? reject(new Error(`${program} interrupted: ${signal}`)) : resolve3(code ?? 1));
  });
}
async function releaseCli(args) {
  const { values, positionals } = parseArgs({ args, allowPositionals: true, options: { root: { type: "string" }, source: { type: "string" }, output: { type: "string" }, id: { type: "string" }, version: { type: "string" }, revision: { type: "string" }, service: { type: "string", default: "cortico-box.service" }, "native-target": { type: "string" }, "data-epoch": { type: "string" }, "legacy-health": { type: "boolean" }, help: { type: "boolean" } } });
  if (values.help) return help;
  const [action] = positionals;
  if (positionals.length !== 1 || !["pack", "install", "init", "upgrade-controller", "status", "list", "verify", "activate", "rollback", "recover"].includes(action)) throw new Error(help);
  if (action === "pack") {
    if (!values.source || !values.output || !values.id || !values.version || !values.revision) throw new Error(help);
    if (values["native-target"] && !["linux-x64", "linux-arm64"].includes(values["native-target"])) throw new Error("Invalid native target");
    const dataEpoch = values["data-epoch"] === void 0 ? void 0 : Number(values["data-epoch"]);
    if (values["native-target"] && dataEpoch === void 0 || dataEpoch !== void 0 && (!Number.isSafeInteger(dataEpoch) || dataEpoch < 1)) throw new Error("Native releases require an explicit positive data epoch");
    const runtime = values["native-target"] ? { kind: "native", platform: "linux", arch: values["native-target"].slice(6) } : void 0;
    return packRelease(resolve2(values.source), resolve2(values.output), { id: values.id, version: values.version, revision: values.revision, healthProtocol: values["legacy-health"] ? 0 : 1, ...runtime ? { runtime } : {}, ...dataEpoch === void 0 ? {} : { dataEpoch } });
  }
  const root = values.root ?? process.env.CORTICO_TERRARIUM_DEPLOY_ROOT;
  if (!root) throw new Error("--root is required");
  const store = new ReleaseStore(root);
  if (action === "status") return store.state();
  if (action === "list") return store.list();
  if (action === "verify") {
    if (!values.id) throw new Error("--id is required");
    return verifyRelease(store.directory(values.id));
  }
  if (process.env.CORTICO_TERRARIUM_RELEASE_LOCKED !== "1") {
    const lock = join2(store.root, ".deployment/lock");
    await mkdir2(dirname2(lock), { recursive: true, mode: 448 });
    const handle = await open2(lock, "a", 384);
    await handle.close();
    const code2 = await command("flock", ["--nonblock", "--conflict-exit-code", "75", lock, process.execPath, fileURLToPath(import.meta.url), ...args], { ...process.env, CORTICO_TERRARIUM_RELEASE_LOCKED: "1" });
    if (code2 !== 0) throw new Error(code2 === 75 ? "Another release operation holds the deployment lock" : `Release operation failed (${code2})`);
    return void 0;
  }
  if (action === "install") {
    if (!values.source) throw new Error("--source is required");
    return store.install(resolve2(values.source));
  }
  if (action === "upgrade-controller") {
    if (!values.id) throw new Error("--id is required");
    await store.upgradeController(values.id);
    return store.state();
  }
  if (action === "init") {
    if (!values.id) throw new Error("--id is required");
    await store.initialize(values.id);
    return store.state();
  }
  if (!/^[a-zA-Z0-9][a-zA-Z0-9_.@-]*\.service$/.test(values.service)) throw new Error("Invalid user service name");
  const pending = action === "recover" ? (await store.state()).pending : await store.queue(values.id, action === "rollback" ? "rollback" : "activate");
  if (action === "recover" && !pending) return store.state();
  const code = await command("systemctl", ["--user", "restart", values.service]);
  if (code !== 0) throw new Error("Could not restart the runner; the journal is retained. Run recover after correcting the service.");
  const deadline = Date.now() + 2 * Number(process.env.CORTICO_TERRARIUM_START_TIMEOUT_MS ?? 2e4) + 15e3;
  while (Date.now() < deadline) {
    const state = await store.state();
    if (!state.pending) {
      const result = state.history.find((item) => item.id === pending?.id);
      if (result?.status === "failed") throw new Error(`Activation failed; restored ${state.current}: ${result.error}`);
      return state;
    }
    await delay(200);
  }
  throw new Error("Runner has not finished the transaction. Inspect status and service logs; the journal is retained.");
}
if (process.argv[1] && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) void releaseCli(process.argv.slice(2)).then((result) => {
  if (result !== void 0) console.log(typeof result === "string" ? result : JSON.stringify(result, null, 2));
}).catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
export {
  releaseCli
};
