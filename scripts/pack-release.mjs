// Owner: src/main.rs, controller/release.mjs. Build a schema 2 payload for the pinned release controller.
import {execFileSync} from 'node:child_process';
import {cp,mkdtemp,mkdir,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
const {values}=parseArgs({options:{binary:{type:'string'},output:{type:'string'},id:{type:'string'},revision:{type:'string'},version:{type:'string',default:'0.1.0'}}});
if(!values.output||!values.id||!values.revision)throw Error('--output <new-directory> --id <release-id> --revision <git-sha> [--binary <path>] [--version <server-version>]');
if(process.platform!=='linux'||!['x64','arm64'].includes(process.arch))throw Error('Build the Native payload on its Linux target architecture');
const repository=fileURLToPath(new URL('..',import.meta.url));
const source=await mkdtemp(join(tmpdir(),'terrarium-release-'));
try{
 await cp(resolve(values.binary??join(repository,'target/release/terrarium-server')),join(source,'server'));
 for(const name of ['admin.mjs','runner.mjs','release.mjs'])await cp(join(repository,'controller',name),join(source,name));
 for(const name of ['index.html','scene.js'])await cp(join(repository,'web',name),join(source,name));
 await cp(join(repository,'web/admin'),join(source,'console'),{recursive:true});
 await mkdir(resolve(values.output,'..'),{recursive:true});
 const raw=execFileSync(process.execPath,[join(repository,'controller/release.mjs'),'pack','--source',source,'--output',resolve(values.output),'--id',values.id,'--version',values.version,'--revision',values.revision,'--native-target',`linux-${process.arch}`,'--data-epoch','2'],{encoding:'utf8'});
 const manifest=JSON.parse(raw);
 console.log(JSON.stringify({id:manifest.id,dataEpoch:manifest.dataEpoch,runtime:manifest.runtime,files:manifest.files.length,bytes:manifest.files.reduce((sum,file)=>sum+file.size,0)}));
}finally{await rm(source,{recursive:true,force:true});}
