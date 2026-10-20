// Owner: scripts/migrate-storage.mjs. Manual rehearsal using a temporary user systemd service and synthetic data.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {createServer} from 'node:net';
import {mkdtemp,mkdir,cp,readFile,writeFile,rm} from 'node:fs/promises';
import {homedir} from 'node:os';
import {join,dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';
const [legacyPath,nativePath]=process.argv.slice(2);
if(!legacyPath||!nativePath)throw Error('Supply installed TS and Native release directories');
const script=fileURLToPath(new URL('./migrate-storage.mjs',import.meta.url));
const fixtures=JSON.parse(await readFile(new URL('../tests/fixtures/reports.json',import.meta.url),'utf8'));
const cache=join(homedir(),'.cache');await mkdir(cache,{recursive:true});
const root=await mkdtemp(join(cache,'terrarium-migration-'));
const service=`terrarium-migration-${randomUUID()}.service`;
const unit=join(process.env.XDG_CONFIG_HOME??join(homedir(),'.config'),'systemd/user',service);
function run(program,args){return execFileSync(program,args,{encoding:'utf8',timeout:90000,stdio:['ignore','pipe','pipe']});}
const system=action=>run('systemctl',['--user',action,...(action==='daemon-reload'?[]:[service])]);
async function port(){const socket=createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));const port=socket.address().port;await new Promise(r=>socket.close(r));return port;}
const publicPort=await port(),privatePort=await port(),url=`http://127.0.0.1:${publicPort}`,admin=`http://127.0.0.1:${privatePort}`;
const health=url+'/health';
async function request(base,path,body,token){const r=await fetch(base+path,{method:body===undefined?'GET':path==='/api/reports'?'PUT':'POST',headers:{...(token?{authorization:`Bearer ${token}`}:{ }),'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(10000)});const parsed=await r.json();assert.ok(r.ok,`HTTP ${r.status}: ${parsed.error??path}`);return parsed;}
async function ready(){for(let i=0;i<100;i++){try{return await request(url,'/health');}catch{}await delay(100);}throw Error('Fixture service did not start');}
const native=JSON.parse(await readFile(join(nativePath,'release.json'),'utf8'));
const legacy=JSON.parse(await readFile(join(legacyPath,'release.json'),'utf8'));
const stateFor=kind=>({...structuredClone(fixtures[kind]),name:'Synthetic cutover fixture',updatedAtMs:Date.now()});
try{
 await mkdir(join(root,'versions'),{recursive:true});await mkdir(join(root,'data'),{mode:0o700});
 await cp(legacyPath,join(root,'versions',legacy.id),{recursive:true});
 for(const name of ['leaderboard.json','credentials.json'])await writeFile(join(root,'data',name),'{}',{mode:0o600});
 const release=join(resolve(nativePath),'release.mjs');
 run(process.execPath,[release,'install','--root',root,'--source',resolve(nativePath)]);
 run(process.execPath,[release,'init','--root',root,'--id',legacy.id]);
 run(process.execPath,[release,'upgrade-controller','--root',root,'--id',native.id]);
 await mkdir(dirname(unit),{recursive:true});
 await writeFile(unit,`[Service]\nWorkingDirectory=${root}\nExecStart=${process.execPath} ${root}/manager/runner.mjs\nEnvironment=CORTICO_TERRARIUM_DEPLOY_ROOT=${root}\nEnvironment=PORT=${publicPort}\nEnvironment=TERRARIUM_DATA_FILE=${root}/data/leaderboard.json\nEnvironment=TERRARIUM_CREDENTIALS_FILE=${root}/data/credentials.json\nEnvironment=CORTICO_TERRARIUM_ADMIN_HOST=127.0.0.1\nEnvironment=CORTICO_TERRARIUM_ADMIN_PORT=${privatePort}\nEnvironment=CORTICO_TERRARIUM_ADMIN_ALLOWED_CIDR=127.0.0.1/32\nKillMode=control-group\nTimeoutStopSec=10\n`);
 system('daemon-reload');system('start');await ready();
 const first=await request(url,'/api/registrations',{});
 await request(url,'/api/reports',{id:first.id,state:stateFor('land'),clientVersion:'0.8.6'},first.token);
 await request(url,'/api/feedback',{id:first.id,text:'Synthetic before migration'},first.token);
 let token=(await readFile(join(root,'data/admin-token'),'utf8')).trim();
 const announce=text=>request(admin,'/api/admin/announcements',{text,targets:[first.id]},token);
 const before=await announce('Synthetic before migration');
 const args=['--root',root,'--service',service,'--health-url',health,'--apply'];
 run(process.execPath,[script,'migrate','--id',native.id,...args]);
 const migrated=await ready();assert.equal(migrated.implementation,'rust');
 const next=stateFor('land');next.name='Synthetic after migration';
 await request(url,'/api/reports',{id:first.id,state:next,clientVersion:'0.8.6'},first.token);
 const second=await request(url,'/api/registrations',{kind:'water'});
 await request(url,'/api/reports',{id:second.id,state:stateFor('water'),clientVersion:'0.8.6'},second.token);
 await request(url,'/api/feedback',{id:first.id,text:'Synthetic after migration'},first.token);
 const after=await announce('Synthetic after migration');assert.ok(after.id>before.id);
 run(process.execPath,[script,'rollback',...args]);
 const restored=await ready();assert.equal(restored.releaseId,legacy.id);
 assert.equal((await request(url,`/api/boxes/${first.id}`)).state.name,next.name);
 assert.equal((await request(url,`/api/boxes/${second.id}`)).kind,'water');
 const credentials=JSON.parse(await readFile(join(root,'data/credentials.json'),'utf8'));
 assert.equal(credentials[first.id],first.token);assert.equal(credentials[second.id],second.token);
 const feedback=await request(admin,'/api/admin/feedback',undefined,token);assert.equal(feedback.feedback.length,2);
 const notices=await request(admin,'/api/admin/announcements',undefined,token);assert.deepEqual(notices.announcements.map(n=>n.id),[before.id,after.id]);
 // A missing validation runtime makes the Native listener fail before it can accept reports.
 await mkdir(unit+'.d',{recursive:true});await writeFile(join(unit+'.d','missing-validator.conf'),'[Service]\nEnvironment=CORTICO_TERRARIUM_LEGACY_NODE=/nonexistent/fixture-node\n');system('daemon-reload');
 let failed=false;try{run(process.execPath,[script,'migrate','--id',native.id,...args]);}catch{failed=true;}
 assert.equal(failed,true);assert.equal((await ready()).releaseId,legacy.id);
 assert.equal((await request(url,`/api/boxes/${first.id}`)).state.name,next.name);
 assert.equal(JSON.parse(await readFile(join(root,'.deployment/storage-migration.json'),'utf8')).phase,'rolledback');
 // An interrupted import cannot start either implementation until recovery completes.
 const journalFile=join(root,'.deployment/storage-migration.json');
 const savedJournal=JSON.parse(await readFile(journalFile,'utf8'));
 system('stop');
 await writeFile(journalFile,JSON.stringify({...savedJournal,phase:'imported',exposed:false}));
 let blocked=false;try{system('start');}catch{blocked=true;}
 assert.equal(blocked,true);
 await writeFile(journalFile,JSON.stringify(savedJournal));
 system('start');await ready();
 console.log('Interrupted imports are blocked by ExecStartPre until migration recovery');
 console.log('Systemd rehearsal passed: TS → Native → latest-data rollback; credentials, states, feedback and notice IDs retained');
 console.log('Failed Native startup automatically restores the TS release without losing committed writes');
}finally{
 try{system('stop');}catch{}
 await rm(unit,{force:true});await rm(unit+'.d',{recursive:true,force:true});system('daemon-reload');
 try{system('reset-failed');}catch{}
 // Release directories are sealed read-only by the controller.
 run('chmod',['-R','u+w',root]);await rm(root,{recursive:true,force:true});
}
