// Owner: src/http.rs, src/store.rs. Manual comparison on an isolated copy; no records are printed.
import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,cp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {createInterface} from 'node:readline';
const [legacyFile,snapshot,binaryArg]=process.argv.slice(2);
if(!legacyFile||!snapshot) throw Error('Supply a legacy server.mjs and a snapshot directory');
for(const key of Object.keys(process.env)) if(key.startsWith('CORTICO_TERRARIUM_')||key.startsWith('TERRARIUM_')) delete process.env[key];
const {start}=await import(pathToFileURL(resolve(legacyFile)).href);
const binary=binaryArg??fileURLToPath(new URL('../target/debug/terrarium-server',import.meta.url));
const dir=await mkdtemp(join(tmpdir(),'terrarium-snapshot-'));
const database=join(dir,'data.sqlite');
let legacy,rust;
const get=async(base,path,token)=>{const r=await fetch(base+path,{headers:token?{authorization:`Bearer ${token}`}:{},signal:AbortSignal.timeout(10000)});return{status:r.status,body:await r.json()};};
try{
 const copy=join(dir,'legacy');await cp(snapshot,copy,{recursive:true});
 const imported=spawnSync(binary,['import-legacy','--from',copy,'--database',database,'--apply'],{encoding:'utf8'});
 assert.equal(imported.status,0,imported.stderr);
 legacy=await start({dataFile:join(copy,'leaderboard.json'),credentialsFile:join(copy,'credentials.json'),adminTokenFile:join(copy,'admin-token'),port:0});
 const old=`http://127.0.0.1:${legacy.address().port}`;
 rust=spawn(binary,['serve','--database',database,'--listen','127.0.0.1:0','--admin-listen','127.0.0.1:0','--enable-writes'],{stdio:['ignore','pipe','inherit']});
 let url,admin;const lines=createInterface({input:rust.stdout});
 for await(const line of lines){const a=/Private admin on (\S+)/.exec(line);if(a)admin='http://'+a[1];const p=/Rust preview on (\S+)/.exec(line);if(p){url='http://'+p[1];lines.close();break;}}
 assert.ok(url&&admin);
 const token=(await readFile(join(copy,'admin-token'),'utf8')).trim();
 const entries=JSON.parse(await readFile(join(copy,'leaderboard.json'),'utf8'));
 let comparisons=0;
 const compare=async(path,privateApi=false)=>{const values=await Promise.all([old,privateApi?admin:url].map(base=>get(base,path,privateApi?token:undefined)));try {assert.deepEqual(values[1],values[0]);} catch {throw Error(`Snapshot response differs at ${path.split('?')[0]}`);}comparisons++;};
 await compare('/api/leaderboard');
 for(const id of Object.keys(entries)){await compare('/api/boxes/'+id);await compare('/api/boxes/'+id+'?view=replays');}
 for(const kind of ['land','water']){
  await compare(`/api/leaderboard?kind=${kind}`);
  await compare(`/api/leaderboard?kind=${kind}&view=summary&limit=100`);
  for(const sort of ['score','recent']){
   const first=await get(old,`/api/leaderboard?kind=${kind}&view=gallery&sort=${sort}`);
   for(let page=1;page<=Math.max(1,first.body.pages);page++) await compare(`/api/leaderboard?kind=${kind}&view=gallery&sort=${sort}&page=${page}`);
  }
 }
 for(const path of ['/api/admin/controls','/api/admin/announcements','/api/admin/feedback'])await compare(path,true);
 await compare('/api/announcements/public');
 const credentials=JSON.parse(await readFile(join(copy,'credentials.json'),'utf8'));
 const beginning=performance.now();let count=0;
 await Promise.all(Array.from({length:16},async()=>{for(let i=0;i<20;i++){const r=await get(url,`/api/leaderboard?kind=${i%2?'land':'water'}&view=gallery&page=${i%4+1}`);assert.equal(r.status,200);count++;}}));
 const duration=performance.now()-beginning;
 console.log(JSON.stringify({boxes:Object.keys(entries).length,credentials:Object.keys(credentials).length,comparisons,concurrentReadRequests:count,readRequestsPerSec:Math.round(count*1000/duration),passed:true}));
}finally{
 if(legacy)await new Promise((resolve,reject)=>legacy.close(e=>e?reject(e):resolve()));
 if(rust){const exit=once(rust,'exit');rust.kill('SIGTERM');await exit;}
 await rm(dir,{recursive:true,force:true});
}
