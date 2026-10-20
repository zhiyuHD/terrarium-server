import {mkdtemp,writeFile,mkdir,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';import {join}from'node:path';import {spawn,spawnSync}from'node:child_process';import {once}from'node:events';import {createInterface}from'node:readline';import assert from'node:assert/strict';
import {pathToFileURL,fileURLToPath}from'node:url';
import{resolve}from'node:path';
const root=process.argv[2];if(!root)throw Error('Supply the World package directory');
const load=file=>import(pathToFileURL(resolve(root,'src',file)).href);
const {LeaderboardClient}=await load('leaderboard.ts');const {newState}=await load('state.ts');const {newAquarium}=await load('aquarium.ts');
const dir=await mkdtemp(join(tmpdir(),'terrarium-rust-client-'));const binary=process.argv[3]??fileURLToPath(new URL('../target/debug/terrarium-server',import.meta.url));const database=join(dir,'data.sqlite');
let server;const clients=[];
async function start(){server=spawn(binary,['serve','--database',database,'--listen','127.0.0.1:0','--enable-writes'],{stdio:['ignore','pipe','inherit']});const lines=createInterface({input:server.stdout});for await(const line of lines){const address=/Rust preview on (\S+)/.exec(line);if(address){lines.close();return 'http://'+address[1];}}throw Error('No listen address');}
try{
 const snapshot=join(dir,'snapshot');await mkdir(snapshot);for(const name of['leaderboard.json','credentials.json'])await writeFile(join(snapshot,name),'{}');const imported=spawnSync(binary,['import-legacy','--from',snapshot,'--database',database,'--apply'],{encoding:'utf8'});assert.equal(imported.status,0,imported.stderr);
 const url=await start();
 for(const kind of['land','water']){const state=kind==='land'?newState(Date.now()):newAquarium(Date.now());const client=new LeaderboardClient({url,id:'',automaticIds:true,kind,credentialsFile:join(dir,kind,'report-credentials.json'),incrementalReports:true});clients.push(client);
 await client.report(state,'day');assert.equal(client.status().state,'synced');assert.equal(client.status().transport,'ws');const id=client.id;
 if(kind==='land'){
 state.plants[0]={...state.plants[0],species:'sunflower',growthHours:30,stage:3,seedRounds:1};state.plants[1].species='mint';
 state.bugs=[{id:'bug-neighbor-protocol',species:'ladybug',x:1,y:1,ageHours:1,energy:80,hunger:10}];
 state.food=[{id:'food-neighbor-protocol',kind:'sunflower_seed',x:0,y:0,portions:3,droppedAtMs:state.updatedAtMs}];
 await client.report(state,'day');assert.equal(client.status().state,'synced');
 const accepted=await(await fetch(`${url}/api/boxes/${id}`)).json();assert.deepEqual(accepted.state,state);
 console.log('New land species and sunflower seed rounds accepted');
 }
 state.name=kind+' mirror garden';state.updatedAtMs=Date.now();await client.report(state,'night');assert.equal(client.status().state,'synced');
 const uploaded=await(await fetch(`${url}/api/boxes/${id}`)).json();assert.equal(uploaded.state.name,state.name);assert.equal(uploaded.kind,kind);assert.equal(uploaded.reportTransport,'websocket');
 const ranking=await client.leaderboard(10);assert.equal(ranking.own?.id,id);assert.equal(ranking.total,1);const notices=await client.announcements(0);assert.deepEqual(notices.announcements,[]);
 client.close();const reconnected=new LeaderboardClient({url,id:'',automaticIds:true,kind,credentialsFile:join(dir,kind,'report-credentials.json'),incrementalReports:true});clients.push(reconnected);await reconnected.restore();assert.equal(reconnected.id,id);await reconnected.report(state,'day');assert.equal(reconnected.id,id);assert.equal(reconnected.status().state,'synced');
 console.log(JSON.stringify({kind,registered:true,full:true,delta:true,transport:client.status().transport,ranking:true,announcements:true,reconnect:true,credentialsReused:true}));
 }
 for(const client of clients)client.close();const exited=once(server,'exit');server.kill('SIGTERM');const[code,signal]=await exited;assert.equal(code,0);assert.equal(signal,null);server=undefined;console.log('SIGTERM exits successfully');
 const exported=spawnSync(binary,['export-legacy','--database',database,'--output',join(dir,'export')],{encoding:'utf8'});assert.equal(exported.status,0,exported.stderr);const credentials=JSON.parse(await readFile(join(dir,'export','credentials.json'),'utf8'));assert.equal(Object.keys(credentials).length,2);console.log('Export retains exactly two identities');
}finally{for(const client of clients)client.close();server?.kill('SIGTERM');await rm(dir,{recursive:true,force:true});}
