import assert from 'node:assert/strict';
import {readFileSync,existsSync,readdirSync} from 'node:fs';
import {resolve} from 'node:path';
const root=resolve('dist');
const {basePath,origin,isPublic}=JSON.parse(readFileSync(resolve(root,'site-config.json')));
const routes=JSON.parse(readFileSync(resolve(root,'routes.json')));
const siteOrigin=origin||'https://preview.invalid';
let links=0;
function checkURL(value,from,hashCheck=true) {
  if(/^(https?:|data:|mailto:|tel:|\/\/)/.test(value))return;
  const url=new URL(value,siteOrigin+basePath+from);
  assert(url.pathname.startsWith(basePath+'/'),'Link escapes site base path: '+value);
  const relative=decodeURIComponent(url.pathname.slice(basePath.length));
  const target=resolve(root,'.'+relative,relative.endsWith('/')?'index.html':'');
  assert(existsSync(target),'Missing link target: '+value+' in '+from);
  if(url.hash&&hashCheck)assert(readFileSync(target,'utf8').includes('id="'+url.hash.slice(1)+'"'),'Missing anchor: '+value);
  links++;
}
for(const route of routes) {
  const file=resolve(root,route==='/404.html'?'404.html':'.'+route+'/index.html');
  const html=readFileSync(file,'utf8');
  assert(html.includes('lang="ru"')&&html.includes('<h1>')&&html.includes('<title>'),'Missing document metadata: '+route);
  for(const [,value] of html.matchAll(/(?:href|src)="([^"]+)"/g))checkURL(value,route);
  if(isPublic&&route!=='/404.html') {
    assert(html.includes('rel="canonical" href="'+origin+basePath+route+'"'),'Incorrect canonical URL');
    assert(html.includes('content="index, follow"'),'Public pages must be indexable');
  }
}
for(const file of readdirSync(root).filter(name=>name.endsWith('.css'))) {
  const css=readFileSync(resolve(root,file),'utf8');
  for(const [,value] of css.matchAll(/url\(['"]?([^)'"\s]+)['"]?\)/g))checkURL(value,'/'+file,false);
}
assert(existsSync(resolve(root,'.nojekyll')));
assert(!existsSync(resolve(root,'.sites-runtime')));
const sitemap=readFileSync(resolve(root,'sitemap.xml'),'utf8');
assert(!sitemap.includes('404.html'));
if(isPublic)for(const route of routes.filter(route=>route!=='/404.html'))assert(sitemap.includes('<loc>'+origin+basePath+route+'</loc>'));
console.log('OK: '+routes.length+' pages, '+links+' local links/assets, base path '+(basePath||'/')+', '+(isPublic?'public':'preview')+' metadata.');
