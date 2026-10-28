import {build} from 'esbuild';
import {readFile,writeFile,readdir} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=dirname(fileURLToPath(import.meta.url));
const licenses=[];
for(const name of ['markdown-it','argparse','entities','linkify-it','mdurl','punycode.js','uc.micro']){
 const dir=join(root,'node_modules',name);
 const file=(await readdir(dir)).find(file=>/^licen[sc]e(?:\.|$)/i.test(file));
 if(file)licenses.push(name+'\n'+await readFile(join(dir,file),'utf8'));
}
await writeFile(join(root,'../web/forum-markdown.LICENSE.txt'),licenses.join('\n\n'));
await build({absWorkingDir:root,entryPoints:['client.js'],bundle:true,minify:true,format:'iife',globalName:'ForumMarkdown',target:'es2020',outfile:'../web/forum-markdown.js',legalComments:'eof',footer:{js:'/*! Dependency licenses:\n'+licenses.join('\n\n').replaceAll('*/','* /')+'\n*/'}});
const output=join(root,'../web/forum-markdown.js');
await writeFile(output,(await readFile(output,'utf8')).replace(/<\/script/gi,'<\\/script'));
