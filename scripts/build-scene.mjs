// Owner: web/scene.js. Build the observation adapter and renderer into one browser module.
import {createRequire} from 'node:module';
import {parseArgs} from 'node:util';
import {resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
const {values}=parseArgs({options:{world:{type:'string'},renderer:{type:'string'}}});
if(!values.world||!values.renderer)throw Error('--world <World checkout> --renderer <renderer checkout>');
const root=fileURLToPath(new URL('..',import.meta.url));
const require=createRequire(new URL('../photo/package.json',import.meta.url));
const {build}=require('esbuild');
const renderer=join(resolve(values.renderer),'src/index.ts');
await build({stdin:{contents:`export * from ${JSON.stringify(join(resolve(values.world),'webui/scene.ts'))};\nexport {setSceneActive,setSceneFrameRate,renderSceneFrame} from ${JSON.stringify(renderer)};`,resolveDir:root,sourcefile:'observation-scene.ts'},alias:{'terrarium-renderer':renderer},bundle:true,format:'esm',platform:'browser',target:'es2022',outfile:join(root,'web/scene.js')});
