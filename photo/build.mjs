// Owner: photo/render.mjs. The payload uses the public observation renderer.
import {build} from 'esbuild';
import {cp, mkdir} from 'node:fs/promises';
import {createRequire} from 'node:module';
const require = createRequire(import.meta.url);
await mkdir(new URL('./dist/', import.meta.url), {recursive:true});
await build({entryPoints:[new URL('./render.mjs',import.meta.url).pathname],bundle:true,external:['./photo-canvas.node'],platform:'node',format:'esm',target:'node22',outfile:new URL('./dist/photo-worker.mjs',import.meta.url).pathname,banner:{js:'import {createRequire as photoRequire} from "node:module"; const require = photoRequire(import.meta.url); const __dirname = new URL(".", import.meta.url).pathname;'},plugins:[{name:'native-canvas',setup(build){build.onResolve({filter:/^\.\/js-binding$/},()=>({path:'binding',namespace:'photo'}));build.onLoad({filter:/.*/,namespace:'photo'},()=>({contents:'module.exports = require("./photo-canvas.node");',loader:'js'}));}}]});
const binding = require.resolve(`@napi-rs/canvas-linux-${process.arch}-gnu`);
await cp(binding,new URL('./dist/photo-canvas.node',import.meta.url));
for(const name of ['MiSans-Regular.woff2','MiSans-Semibold.woff2','MiSans-LICENSE.pdf'])await cp(new URL(`../web/admin/assets/${name}`,import.meta.url),new URL(`./dist/${name}`,import.meta.url));

await cp(new URL('./node_modules/@napi-rs/canvas/LICENSE',import.meta.url),new URL('./dist/Photo-canvas-LICENSE.txt',import.meta.url));
