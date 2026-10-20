// Owner: src/main.rs, src/store.rs. Cross-epoch migrations stop writers and retain an export-based rollback.
import { spawn, execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { constants } from 'node:fs';
import { access, lstat, mkdir, open, readFile, rename, rm, symlink } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { setTimeout as delay } from 'node:timers/promises';
import assert from 'node:assert/strict';

const documents = ['leaderboard.json', 'credentials.json', 'controls.json', 'announcements.json', 'feedback.json', 'protocol-notices.json', 'admin-token'];
const { values, positionals } = parseArgs({ allowPositionals: true, options: {
  root: {type:'string'}, id: {type:'string'}, service: {type:'string', default:'cortico-box.service'},
  'health-url': {type:'string', default:'http://127.0.0.1:8787/health'}, apply: {type:'boolean'}, locked: {type:'boolean'},
}});
const action = positionals[0];
if (!values.root || positionals.length !== 1 || !['migrate', 'rollback', 'guard'].includes(action)) throw Error('migrate|rollback --root <deployment> [--id <native-release>] [--apply] [--service <user.service>] [--health-url <loopback-url>]');
const root = resolve(values.root), service = values.service;
if (!/^[a-zA-Z0-9][a-zA-Z0-9_.@-]*\.service$/.test(service)) throw Error('Invalid user service');
const health = new URL(values['health-url']);
if (health.protocol !== 'http:' || health.hostname !== '127.0.0.1' || health.pathname !== '/health') throw Error('Health URL must use loopback HTTP /health');
const stateFile = join(root, '.deployment/state.json');
const journalFile = join(root, '.deployment/storage-migration.json');
const database = join(root, 'data/terrarium.sqlite');
const json = async file => JSON.parse(await readFile(file, 'utf8'));
async function exists(file) { try {await access(file, constants.F_OK); return true;} catch (error) {if(error.code==='ENOENT') return false; throw error;} }
async function durable(file, bytes, mode=0o600) {
  await mkdir(dirname(file), {recursive:true, mode:0o700});
  const temp = `${file}.${randomUUID()}.tmp`;
  const handle = await open(temp, 'wx', mode);
  try {await handle.writeFile(bytes); await handle.sync();} finally {await handle.close();}
  try {await rename(temp, file); const dir = await open(dirname(file), 'r'); try {await dir.sync();} finally {await dir.close();}}
  finally {await rm(temp, {force:true});}
}
const save = (file, value) => durable(file, JSON.stringify(value, null, 2)+'\n');
function run(program, args) { return execFileSync(program, args, {encoding:'utf8', timeout:60000, maxBuffer:4*1024*1024, stdio:['ignore','pipe','pipe']}); }
function system(action) { run('systemctl', ['--user', action, service]); }
async function manifest(id) {
  if (!/^[a-z0-9][a-z0-9._-]{0,79}$/.test(id ?? '')) throw Error('Invalid release ID');
  const verified = JSON.parse(run(process.execPath, [join(root,'manager/release.mjs'), 'verify', '--root', root, '--id', id]));
  assert.equal(verified.id, id);
  return verified;
}
async function switchTo(state) {
  const temp = join(root, `.current-${randomUUID()}`);
  await symlink(`versions/${state.current}`, temp);
  await rename(temp, join(root, 'current'));
  await save(stateFile, state);
}
async function healthy(id) {
  const deadline = Date.now()+30000;
  while (Date.now()<deadline) {
    try {
      const response = await fetch(health, {signal:AbortSignal.timeout(1000), redirect:'error'});
      const body = await response.json();
      if (response.ok && body.ok === true && body.releaseId === id) return body;
    } catch {}
    await delay(100);
  }
  throw Error(`Service health did not match ${id}`);
}
async function snapshot(output) {
  await mkdir(output, {mode:0o700});
  for (const name of documents) {
    const source = join(root, 'data', name);
    if (!await exists(source)) continue;
    assert.equal((await lstat(source)).isFile(), true, 'Data inputs must be regular files');
    await durable(join(output,name), await readFile(source));
  }
}
async function checkExport(snapshot, exported) {
  for (const name of documents) {
    if (!await exists(join(snapshot,name))) continue;
    const before = await readFile(join(snapshot,name));
    const after = await readFile(join(exported,name));
    if (name.endsWith('.json')) assert.deepEqual(JSON.parse(after), JSON.parse(before), `Export mismatch: ${name}`);
    else assert.deepEqual(after, before, `Export mismatch: ${name}`);
  }
}
function result(journal, rollback) {
  return {id:randomUUID(), action:rollback?'rollback':'activate', from:rollback?journal.to:journal.fromState.current,
    to:rollback?journal.fromState.current:journal.to, status:'succeeded', finishedAt:new Date().toISOString()};
}
async function installGuard() {
  const script=join(root,'.deployment/storage-migration.mjs');
  await durable(script, await readFile(fileURLToPath(import.meta.url)));
  const directory=join(process.env.XDG_CONFIG_HOME??join(homedir(),'.config'),'systemd/user',service+'.d');
  const quote=value=>JSON.stringify(value.replaceAll('%','%%'));
  await durable(join(directory,'storage-migration.conf'),`[Service]\nExecStartPre=${[process.execPath,script,'guard','--root',root].map(quote).join(' ')}\n`);
  run('systemctl',['--user','daemon-reload']);
}
async function startupGuard() {
  if (!await exists(journalFile)) return;
  const journal=await json(journalFile), state=await json(stateFile);
  if (['running','rolledback'].includes(journal.phase)) return;
  if (journal.phase==='switched' && journal.exposed===true && state.current===journal.to) return;
  throw Error('Storage migration is incomplete; recover with migrate-storage rollback before starting the service');
}
async function restore(journal) {
  system('stop');
  let source = journal.rollbackExport;
  // Once the new listener has been exposed, only an export of its latest committed writes is safe.
  const exposed=journal.exposed;
  journal.phase='restoring';await save(journalFile,journal);
  if (exposed && !source) {
    source = join(journal.backup, `rollback-export-${randomUUID()}`);
    const binary = join(root, 'versions', journal.to, 'server');
    run(binary, ['check','--database',database]);
    run(binary, ['export-legacy','--database',database,'--output',source]);
    journal.rollbackExport=source;await save(journalFile,journal);
  }
  for (const name of source ? documents : []) {
    const file = join(source,name);
    if (await exists(file)) await durable(join(root,'data',name), await readFile(file));
    else await rm(join(root,'data',name), {force:true});
  }
  const restored = {...journal.fromState, pending:null,
    history:[...journal.fromState.history,result(journal,true)].slice(-100)};
  await switchTo(restored);
  if (await exists(database)) {
    const parked = join(journal.backup, 'sqlite-after-rollback');
    await mkdir(parked, {recursive:true,mode:0o700});
    for (const suffix of ['', '-wal', '-shm']) {
      if (await exists(database+suffix)) await rename(database+suffix,join(parked,'terrarium.sqlite'+suffix));
    }
  }
  await save(journalFile, {...journal,phase:'rolledback',finishedAt:new Date().toISOString()});
  system('start');
  await healthy(restored.current);
  console.log('Restored the TS release using the latest committed data');
}
async function execute() {
  if (action==='guard') {await startupGuard();return;}
  const state = await json(stateFile);
  assert.equal(state.pending, null, 'Finish the pending release transaction first');
  if (action==='rollback') {
    const journal = await json(journalFile);
    if (!['running','switched','imported','stopped','prepared','restoring'].includes(journal.phase)) throw Error('No recoverable storage migration');
    assert.ok([journal.to,journal.fromState.current].includes(state.current), 'Current release differs from this migration');
    if (!values.apply) {console.log('Dry run: stop service, export latest SQLite writes, restore JSON and TS release');return;}
    await restore(journal); return;
  }
  const old = await manifest(state.current), next = await manifest(values.id);
  assert.equal(old.dataEpoch,1,'Source must use JSON epoch 1');
  assert.equal(next.dataEpoch,2,'Target must use SQLite epoch 2');
  assert.equal(next.runtime?.kind,'native');
  assert.equal(next.runtime.platform,process.platform);
  assert.equal(next.runtime.arch,process.arch);
  assert.equal(await exists(database),false,'Refusing to replace an existing SQLite database');
  if (await exists(journalFile)) assert.equal((await json(journalFile)).phase,'rolledback','Storage migration already recorded');
  if (!values.apply) {console.log('Dry run: stop service, snapshot JSON, import SQLite, verify export, start Native epoch 2');return;}
  await installGuard();
  const journal = {schema:1,phase:'prepared',fromState:state,to:next.id,exposed:false,
    backup:join(root,'backups',`storage-${Date.now()}-${randomUUID()}`),startedAt:new Date().toISOString()};
  const binary = join(root,'versions',next.id,'server');
  await mkdir(journal.backup,{recursive:true,mode:0o700});
  await save(journalFile,journal);
  try {
    system('stop');
    await snapshot(join(journal.backup,'snapshot'));
    await save(join(journal.backup,'deployment-state.json'),state);
    journal.phase='stopped';await save(journalFile,journal);
    run(binary,['import-legacy','--from',join(journal.backup,'snapshot'),'--database',database,'--apply']);
    run(binary,['check','--database',database]);
    const exported=join(journal.backup,'import-verification');
    run(binary,['export-legacy','--database',database,'--output',exported]);
    await checkExport(join(journal.backup,'snapshot'),exported);
    journal.phase='imported';await save(journalFile,journal);
    journal.phase='switched';journal.exposed=true;await save(journalFile,journal);
    await switchTo({...state,current:next.id,previous:null,pending:null});
    system('start');
    const status=await healthy(next.id);
    assert.equal(status.implementation,'rust');assert.equal(status.readOnly,false);
    await save(stateFile,{...state,current:next.id,previous:null,pending:null,history:[...state.history,result(journal,false)].slice(-100)});
    journal.phase='running';await save(journalFile,journal);
    console.log('Native epoch 2 active; JSON snapshot and export rollback retained');
  } catch(error) {
    if (journal.phase==='prepared') {
      // A failed stop or snapshot must not replace intact JSON files with an incomplete snapshot.
      await switchTo(state);await save(journalFile,{...journal,phase:'rolledback'});
      system('start');await healthy(state.current);
    } else {
      try {await restore(journal);} catch(restoreError) {
        system('stop');
        throw Error(`Migration failed; service stopped because rollback could not preserve data: ${restoreError.message}`, {cause:error});
      }
    }
    throw error;
  }
}
if (values.apply && !values.locked) {
  const child=spawn('flock',['--nonblock','--conflict-exit-code','75',join(root,'.deployment/lock'),process.execPath,fileURLToPath(import.meta.url),...process.argv.slice(2),'--locked'],{stdio:'inherit'});
  child.on('error',error=>{console.error(error.message);process.exitCode=1;});
  child.on('exit',code=>{process.exitCode=code??1;});
} else {
  try {await execute();} catch(error) {console.error(error.message);process.exitCode=1;}
}
