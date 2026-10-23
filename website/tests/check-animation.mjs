import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import assert from 'node:assert/strict';
const source=readFileSync('dist/hero.js','utf8');
function boot(reduced=false,code=source){
  const paths=[];const scanPaths=[];const classes=new Set();let animate,observe;
  const art={classList:{toggle:(name,value)=>value?classes.add(name):classes.delete(name)}};
  const element=()=>({attributes:{},setAttribute(key,value){this.attributes[key]=value;},removeAttribute(key){delete this.attributes[key];}});
  const svg={parentElement:art};const group={...element(),append:p=>paths.push(p)};const silhouette=element();const connector=element();const mask=element();const scanGroup={...element(),append:p=>scanPaths.push(p)};
  const polygon=[[128,165],[211,40],[220,90],[158,176],[130,220],[128,165]];
  silhouette.querySelectorAll=()=>[{getTotalLength:()=>5,getPointAtLength(t){const i=Math.min(4,Math.floor(t));const f=t-i;return {x:polygon[i][0]+(polygon[i+1][0]-polygon[i][0])*f,y:polygon[i][1]+(polygon[i+1][1]-polygon[i][1])*f};}}];
  const document={hidden:false,getElementById:id=>id==='sound-feather'?svg:id==='logo-feather'?silhouette:id==='feather-scan-mask'?mask:id==='feather-connector'?connector:id==='feather-scan-strands'?scanGroup:group,createElementNS:element,addEventListener:()=>{}};
  runInNewContext(code,{document,matchMedia:()=>({matches:reduced,addEventListener:()=>{}}),requestAnimationFrame:fn=>{animate=fn;return 1;},cancelAnimationFrame:()=>{},IntersectionObserver:class{constructor(fn){observe=fn;}observe(){observe([{isIntersecting:true}]);}}});
  return {paths,scanPaths,classes,silhouette,group,step:t=>animate(t)};
}
const active=boot();assert.equal(active.paths.length,54);const wave=active.paths[20].attributes.d;active.step(1);active.step(14001);assert.notEqual(active.paths[20].attributes.d,wave);assert(active.classes.has('is-feather'));for(const path of active.paths)assert(!path.attributes.d.includes('NaN'));
assert.equal(active.silhouette.attributes.opacity,'0');assert.equal(active.group.attributes.opacity,'1');assert.equal(active.scanPaths.length,0,'No additional wave layer');
assert(Number(active.paths[12].attributes.opacity)>0);assert(Number(active.paths[40].attributes.opacity)>0);
assert(active.paths.every(path=>!path.attributes.d.includes('C')),'Wave crests stay vertical throughout the transformation');
active.step(17001);assert.equal(active.silhouette.attributes.opacity,'0');assert(Number(active.paths[40].attributes.opacity)>0);
active.step(30001);assert.equal(active.paths[20].attributes.d,wave,'Wave cycle joins without a jump');
const still=boot(true);assert(still.classes.has('is-feather'));assert.equal(still.silhouette.attributes.opacity,'0');
const scan=boot();scan.step(1);scan.step(6001);
assert.equal(scan.silhouette.attributes.opacity,'0','No feather image overlays the wave');
assert.equal(scan.paths.length,54,'The same bars pass through the feather');
const fixed=boot();fixed.step(1);
const coordinates=path=>path.attributes.d.match(/-?\d+(?:\.\d+)?/g).map(Number);
for(let frame=0;frame<=300;frame++) {
  fixed.step(1+frame*100);
  for(let i=0;i<54;i++) {
    const actual=coordinates(fixed.paths[i]);
    const expected=Number((455+i/53*650).toFixed(2));
    for(let j=0;j<actual.length;j+=2)assert.equal(actual[j],expected,'Every column stays at its original X');
  }
  assert.equal(coordinates(fixed.paths[53])[0]-coordinates(fixed.paths[0])[0],650,'Overall animation width stays constant');
}
const breathe=boot();breathe.step(1);breathe.step(10001);const size=breathe.paths[28].attributes.d;breathe.step(11001);assert.notEqual(breathe.paths[28].attributes.d,size,'Feather breathes through the same wave bars');
const leader=boot();leader.step(1);leader.step(5001);
function center(path){const n=path.attributes.d.match(/-?\d+(?:\.\d+)?/g).map(Number);return (n[1]+n[3])/2;}
const raised=center(leader.paths[44]);leader.step(8001);assert(center(leader.paths[44])>raised,'Tenth bar returns downwards');assert(Math.abs(center(leader.paths[44])-382)<25,'Tenth bar returns near the wave center with a smooth join');
for(const time of [6000,8000,10000]){const thin=boot();thin.step(1);thin.step(time+1);assert(thin.paths.every(p=>Number(p.attributes['stroke-width'])<=5.5),'Strokes never thicken into a solid feather');}
const leftward=boot();
function signal(path,i){const n=path.attributes.d.match(/-?\d+(?:\.\d+)?/g).map(Number);const amplitude=(n[3]-n[1])/2;return (amplitude-12)/Math.pow(Math.sin(Math.PI*i/53),.65)-88;}
const previousSignal=signal(leftward.paths[20],20);leftward.step(1);leftward.step(1+.58/(Math.PI*2/3)*1000);
assert(Math.abs(signal(leftward.paths[19],19)-previousSignal)<.03,'Wave pattern travels to the left');
const smooth=boot();smooth.step(1);let preceding=null;
for(let frame=0;frame<=30*60;frame++) {
  smooth.step(1+frame*1000/60);
  const current=smooth.paths.map(path=>{const n=path.attributes.d.match(/-?\d+(?:\.\d+)?/g).map(Number);const y=n.filter((_,i)=>i%2);return [n[0],Math.min(...y),Math.max(...y)];});
  if(preceding)for(let i=0;i<54;i++)for(let axis=0;axis<3;axis++)assert(Math.abs(current[i][axis]-preceding[i][axis])<20,'No abrupt frame-to-frame jumps at the joins');
  preceding=current;
}
console.log('OK:  one set of 54 bars, no overlay, fixed X positions and width, slower feather transitions, contour flow, leftward movement, seamless loop, reduced motion.');
