// Owner: photo/render.mjs. Exercises the shipped worker and the observation scene together.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
const fixtures=JSON.parse(await readFile(new URL('../tests/fixtures/reports.json',import.meta.url)));
for(const kind of ['land','water'])test(`${kind} captures deterministic scenes and caption changes`,()=>{
 const input={entry:{kind,phase:'day',score:120,state:{...fixtures[kind],name:'试验小院'}},caption:'给今天留个纪念',capturedAt:'2026-10-22T14:00:00.000Z',stateHash:'a'.repeat(64)};
 const render=value=>{
  const result=spawnSync(process.execPath,[new URL('./dist/photo-worker.mjs',import.meta.url).pathname],{input:JSON.stringify(value),maxBuffer:8388608,env:{...process.env,DISABLE_SYSTEM_FONTS_LOAD:'1'}});
  assert.equal(result.status,0,result.stderr.toString());const bytes=result.stdout;
  assert.deepEqual([...bytes.subarray(0,8)],[137,80,78,71,13,10,26,10]);assert.equal(bytes.readUInt32BE(16),1280);assert.equal(bytes.readUInt32BE(20),1600);return bytes;
 };
 const first=render(input);assert.deepEqual(render(input),first);
 assert.notDeepEqual(render({...input,caption:'另一张照片'}),first);
 assert.notDeepEqual(render({...input,entry:{...input.entry,phase:'night'}}),first);
});
