import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = process.argv[2];
if (!root) throw Error('Supply the World package directory');
// Keep deployment environment variables out of both legacy modules and the preview.
for (const key of Object.keys(process.env)) {
  if (key.startsWith('CORTICO_TERRARIUM_') || key.startsWith('TERRARIUM_')) delete process.env[key];
}
const load = file => import(pathToFileURL(resolve(root, file)).href);
const { runAdmin } = await load('server/admin.ts');
const { start: startLegacy } = await load('server/index.ts');
const { LeaderboardClient } = await load('src/leaderboard.ts');
const { newState } = await load('src/state.ts');
const { newAquarium } = await load('src/aquarium.ts');
const binary = process.argv[3] ?? fileURLToPath(new URL('../target/debug/terrarium-server', import.meta.url));
const dir = await mkdtemp(join(tmpdir(), 'terrarium-rust-admin-'));
const database = join(dir, 'data.sqlite');
const tokenFile = join(dir, 'admin-token');
let rust, legacy;
const clients = [];
function command(args) {
  const result = spawnSync(binary, args, { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}
async function stopRust() {
  if (!rust) return;
  const exited = once(rust, 'exit');
  rust.kill('SIGTERM');
  const [code, signal] = await exited;
  assert.equal(code, 0);
  assert.equal(signal, null);
  rust = undefined;
}
try {
  const snapshot = join(dir, 'snapshot');
  await mkdir(snapshot);
  for (const name of ['leaderboard.json', 'credentials.json']) await writeFile(join(snapshot, name), '{}');
  command(['import-legacy', '--from', snapshot, '--database', database, '--apply']);
  rust = spawn(binary, ['serve', '--database', database, '--listen', '127.0.0.1:0', '--admin-listen', '127.0.0.1:0', '--enable-writes'], { stdio: ['ignore', 'pipe', 'inherit'] });
  let url, admin;
  const lines = createInterface({ input: rust.stdout });
  for await (const line of lines) {
    const privateAddress = /Private admin on (\S+)/.exec(line);
    if (privateAddress) admin = `http://${privateAddress[1]}`;
    const publicAddress = /Rust preview on (\S+)/.exec(line);
    if (publicAddress) { url = `http://${publicAddress[1]}`; lines.close(); break; }
  }
  assert.ok(url && admin, 'Both listeners started');
  await writeFile(tokenFile, command(['admin-token', '--database', database]), { mode: 0o600 });
  const adminAt = (base, args) => runAdmin([...args, '--server', base, '--token-file', tokenFile]);
  const adminCommand = args => adminAt(admin, args);
  assert.equal((await fetch(`${url}/api/admin/controls`)).status, 404);
  assert.equal((await fetch(admin)).status, 200);
  const ids = [];
  for (const kind of ['land', 'water']) {
    const state = kind === 'land' ? newState(Date.now()) : newAquarium(Date.now());
    const client = new LeaderboardClient({ url, id: '', automaticIds: true, kind, credentialsFile: join(dir, kind, 'credentials.json'), incrementalReports: true });
    clients.push(client);
    await client.report(state, 'day');
    ids.push(client.id);
  }
  const receipt = await clients[0].feedback({ text: 'Synthetic migration feedback', contact: 'offline fixture' }, 'ai');
  const anonymous = await fetch(`${url}/api/feedback`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: 'Synthetic anonymous feedback' }) });
  assert.equal(anonymous.status, 201);
  const broadcast = await adminCommand(['publish', '--title', 'Migration fixture', '--text', 'Synthetic broadcast']);
  const targeted = await adminCommand(['publish', '--text', 'Synthetic private notice', '--to', ids[0]]);
  const revoked = await adminCommand(['publish', '--text', 'Synthetic revoked notice']);
  assert.deepEqual(await adminCommand(['revoke', String(revoked.id)]), { removed: true });
  assert.deepEqual(await adminCommand(['feedback-resolve', String(receipt.id)]), { resolved: true });
  await adminCommand(['http-push', 'off', '--at', new Date(Date.now() + 3600000).toISOString()]);
  await adminCommand(['minimum-version', '0.7.4', '--at', new Date(Date.now() + 7200000).toISOString()]);
  await adminCommand(['minimum-version', 'cancel']);
  const controls = await adminCommand(['controls']);
  const feedback = await adminCommand(['feedback-list']);
  const notices = await adminCommand(['list']);
  assert.equal(feedback.feedback.length, 2);
  assert.ok(feedback.feedback[0].resolvedAt);
  assert.deepEqual(notices.announcements.map(a => a.id), [broadcast.id, targeted.id]);
  const pages = await Promise.all(clients.map(c => c.announcements(0)));
  assert.equal(pages[0].announcements.some(a => a.id === targeted.id), true);
  assert.equal(pages[1].announcements.some(a => a.id === targeted.id), false);
  const entries = await Promise.all(ids.map(async id => (await fetch(`${url}/api/boxes/${id}`)).json()));
  for (const client of clients) client.close();
  await stopRust();
  const exported = join(dir, 'export');
  command(['export-legacy', '--database', database, '--output', exported]);
  assert.equal((await readFile(join(exported, 'admin-token'), 'utf8')).trim(), (await readFile(tokenFile, 'utf8')).trim());
  legacy = await startLegacy({ dataFile: join(exported, 'leaderboard.json'), credentialsFile: join(exported, 'credentials.json'), controlsFile: join(exported, 'controls.json'), announcementsFile: join(exported, 'announcements.json'), adminTokenFile: join(exported, 'admin-token'), port: Number(new URL(url).port) });
  const oldUrl = `http://127.0.0.1:${legacy.address().port}`;
  assert.deepEqual(await adminAt(oldUrl, ['controls']), controls);
  assert.deepEqual(await adminAt(oldUrl, ['feedback-list']), feedback);
  assert.deepEqual(await adminAt(oldUrl, ['list']), notices);
  for (let index = 0; index < ids.length; index++) {
    assert.deepEqual(await (await fetch(`${oldUrl}/api/boxes/${ids[index]}`)).json(), entries[index]);
    const client = new LeaderboardClient({ url: oldUrl, id: '', automaticIds: true, kind: index ? 'water' : 'land', credentialsFile: join(dir, index ? 'water' : 'land', 'credentials.json') });
    clients.push(client);
    await client.restore();
    assert.equal(client.id, ids[index]);
    assert.deepEqual(await client.announcements(0), pages[index]);
    const nextReceipt = await client.feedback({ text: 'Synthetic feedback after rollback' }, 'world');
    assert.ok(nextReceipt.id > receipt.id);
  }
  const nextNotice = await adminAt(oldUrl, ['publish', '--text', 'Synthetic announcement after rollback']);
  assert.ok(nextNotice.id > Math.max(...pages.flatMap(p => p.announcements.map(a => a.id)), revoked.id));
  console.log('Legacy admin CLI: announcement CRUD, feedback resolve, scheduled controls passed');
  console.log('Rust export → TS: states, scores, credentials, admin token, controls, feedback and notice cursors retained');
  console.log('TS continues feedback and announcement writes after rollback');
} finally {
  for (const client of clients) client.close();
  await stopRust();
  if (legacy) await new Promise((resolve, reject) => legacy.close(error => error ? reject(error) : resolve()));
  await rm(dir, { recursive: true, force: true });
}
