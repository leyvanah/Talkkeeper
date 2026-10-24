import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import assert from 'node:assert/strict';
const code=readFileSync('dist/theme.js','utf8');
function boot(saved, dark=false) {
  const root={dataset:{}};const attributes={};let click,change;
  const media={matches:dark,addEventListener:(_,fn)=>change=fn};
  const button={setAttribute:(key,value)=>attributes[key]=value,addEventListener:(_,fn)=>click=fn};
  runInNewContext(code,{document:{documentElement:root,addEventListener:(_,fn)=>fn(),getElementById:()=>button},window:{matchMedia:()=>media},localStorage:{getItem:()=>saved,setItem:(_,value)=>saved=value}});
  return {root,attributes,click:()=>click(),system:value=>{media.matches=value;change();},saved:()=>saved};
}
const auto=boot(null);
assert.equal(auto.root.dataset.themeMode,'auto');assert.equal(auto.root.dataset.theme,'light');
auto.system(true);assert.equal(auto.root.dataset.theme,'dark');
auto.click();assert.equal(auto.root.dataset.theme,'light');assert.equal(auto.saved(),'light');
auto.system(false);auto.system(true);assert.equal(auto.root.dataset.theme,'light');
auto.click();assert.equal(auto.root.dataset.theme,'dark');assert.equal(auto.saved(),'dark');
auto.click();assert.equal(auto.root.dataset.themeMode,'auto');assert.equal(auto.saved(),'auto');
auto.system(false);assert.equal(auto.root.dataset.theme,'light');
assert.equal(boot('dark').root.dataset.theme,'dark');assert.equal(boot('light',true).root.dataset.theme,'light');assert.equal(boot('invalid',true).root.dataset.themeMode,'auto');
console.log('OK: automatic theme, system changes, manual overrides, persistence, existing preferences.');
