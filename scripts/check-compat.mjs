// Owner: src/http.rs. Run manually against isolated servers seeded from the same synthetic snapshot.
import assert from 'node:assert/strict';
const [legacy,rust]=process.argv.slice(2);
if(!legacy||!rust)throw Error('Usage: node scripts/check-compat.mjs <legacy-url> <rust-url>');
const paths=['/api/leaderboard','/api/boxes/box-000','/api/boxes/box-000?view=replays','/api/boxes/missing',...['land','water'].flatMap(kind=>[
 `/api/leaderboard?kind=${kind}`,
 `/api/leaderboard?kind=${kind}&view=gallery`,
 `/api/leaderboard?kind=${kind}&view=gallery&page=2`,
 `/api/leaderboard?kind=${kind}&view=gallery&page=999`,
 `/api/leaderboard?kind=${kind}&view=gallery&sort=recent`,
 `/api/leaderboard?kind=${kind}&view=gallery&q=Garden`,
 `/api/leaderboard?kind=${kind}&view=summary&ownId=box-000`,
 `/api/leaderboard?kind=${kind}&view=summary&limit=2`,
])];
for(const path of paths){const results=await Promise.all([legacy,rust].map(async base=>{const r=await fetch(base+path,{signal:AbortSignal.timeout(10000)});return {status:r.status,body:await r.json()}}));assert.deepEqual(results[1],results[0],path);}
console.log(`${paths.length} read API comparisons passed`);
