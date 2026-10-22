// Owner: src/photos.rs. One frozen snapshot becomes one PNG; no browser process is started.
import {createCanvas, GlobalFonts} from '@napi-rs/canvas';
import {drawScene, createWaterScene} from '../web/scene.js';
import {fileURLToPath} from 'node:url';
GlobalFonts.registerFromPath(fileURLToPath(new URL('./MiSans-Regular.woff2',import.meta.url)), 'MiSans');
GlobalFonts.registerFromPath(fileURLToPath(new URL('./MiSans-Semibold.woff2',import.meta.url)), 'MiSans Bold');
let raw='';
for await (const chunk of process.stdin) {raw+=chunk; if(Buffer.byteLength(raw)>8*1024*1024)throw Error('Photo snapshot too large');}
const {entry,caption,capturedAt,stateHash}=JSON.parse(raw);
const water=entry.kind==='water', state={...entry.state,phase:entry.phase??'day'};
const canvas=createCanvas(1280,1600),g=canvas.getContext('2d');
const colors=water?{paper:'#f0f6fa',ink:'#203f51',primary:'#cce8f1',soft:'#e7e2f4',accent:'#b7dccc'}:{paper:'#f4f7ee',ink:'#25443a',primary:'#d8eacb',soft:'#e9e2f2',accent:'#d0e7e1'};
function card(x,y,w,h,r,color){g.fillStyle=color;g.beginPath();g.roundRect(x,y,w,h,r);g.fill();}
function text(value,x,y,size=24,font='MiSans',color=colors.ink){g.fillStyle=color;g.font=`${size}px "${font}"`;g.fillText(String(value),x,y);}
function lines(value,width,size,font='MiSans'){g.font=`${size}px "${font}"`;const result=[];let line='';for(const char of String(value)){if(char==='\n'||g.measureText(line+char).width>width){result.push(line);line=char==='\n'?'':char;}else line+=char;}result.push(line);return result;}
function wrapped(value,x,y,width,size,lineHeight,maxLines,font='MiSans'){const all=lines(value,width,size,font);for(let i=0;i<Math.min(all.length,maxLines);i++){let line=all[i];if(i===maxLines-1&&all.length>maxLines){while(g.measureText(line+'…').width>width)line=line.slice(0,-1);line+='…';}text(line,x,y+i*lineHeight,size,font);}}
g.fillStyle=colors.paper;g.fillRect(0,0,1280,1600);
card(72,64,264,52,26,colors.primary);text('生态箱 · 照记影踪',96,99,24,'MiSans Bold');
text(water?'水下的一刻':'院子的一刻',76,204,62,'MiSans Bold');
wrapped(state.name||'我的生态箱',78,267,1100,36,46,2,'MiSans Bold');
const date=new Intl.DateTimeFormat('zh-CN',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false}).format(new Date(capturedAt));
text(`${date}  ·  ${state.phase==='night'?'夜间':'白昼'}  ·  UTC+8`,80,366,25);
card(48,411,1184,806,44,colors.primary);
const scene=createCanvas(1600,1000);scene.getContext('2d').scale(2,2);
if(water){
 let frame;
 globalThis.requestAnimationFrame=callback=>{frame=callback;return 1;};
 globalThis.cancelAnimationFrame=()=>{};
 const renderer=createWaterScene(scene);renderer.setState(state);frame(0);renderer.dispose();
}else drawScene(scene,state);
g.save();g.beginPath();g.roundRect(72,435,1136,710,28);g.clip();g.drawImage(scene,72,435,1136,710);g.restore();
text('把此刻留下，把日子慢慢养大。',90,1186,25);
const plants=state.plants?.length??0,animals=(water?state.fish:state.bugs)?.length??0;
const level=Math.max(1,state.progression?.level??1);
const metrics=[['植物',plants], [water?'鱼儿':'虫子',animals], ['积分',Math.round(entry.score??0)], ['扩建等级',level]];
metrics.forEach(([label,value],i)=>{const x=72+i*290;card(x,1241,266,106,28,i%2?colors.soft:colors.accent);text(label,x+24,1276,21);text(value,x+24,1321,34,'MiSans Bold');});
if(caption)wrapped(caption.replace(/\s+/gu,' '),80,1395,1115,28,35,4);
else text('今天也有值得记住的小小生机。',80,1433,30);
text(`拍摄状态 ${stateHash.slice(0,12)}  ·  Cortico Terrarium 0.8.9`,80,1558,19);
process.stdout.write(await canvas.encode('png'));
