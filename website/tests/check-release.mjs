import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {siteSettings,metadata,discovery} from '../metadata.mjs';
import {turns} from '../preview-transcript.mjs';
import {release} from '../release.mjs';

const routes=['/','/download/','/privacy/','/guide/','/404.html'];
const local=siteSettings({});
assert.match(metadata({path:'/',title:'Title',description:'Description',settings:local}),/noindex, nofollow/);
assert.match(discovery(local,routes).robots,/Disallow: \//);
assert.throws(()=>siteSettings({SITE_PUBLIC:'1'}),/SITE_URL/);
assert.throws(()=>siteSettings({SITE_PUBLIC:'1',SITE_URL:'http://example.org'}),/HTTPS/);
const production=siteSettings({SITE_PUBLIC:'1',SITE_URL:'https://example.org'});
const meta=metadata({path:'/guide/',title:'A "quote"',description:'<text>',settings:production});
assert.match(meta,/index, follow/);
assert.match(meta,/canonical" href="https:\/\/example.org\/guide\//);
assert.match(meta,/A &quot;quote&quot;/);
assert.match(meta,/&lt;text>/);
const publicFiles=discovery(production,routes);
assert.equal((publicFiles.sitemap.match(/<url>/g)||[]).length,4);
assert(!publicFiles.sitemap.includes('/404.html'));
assert.match(publicFiles.robots,/Sitemap: https:\/\/example.org\/sitemap.xml/);
assert.match(metadata({path:'/404.html',title:'404',description:'404',settings:production}),/noindex/);
const html=readFileSync('dist/index.html','utf8');
for(const turn of turns)assert.equal(html.split(turn.text).length-1,2,'Both transcript views have the same complete conversation');
assert.equal(turns.at(-1).end,'3:00');
assert(turns.some(turn=>turn.text.includes('до вторника')));
assert(turns.some(turn=>turn.text.includes('17 октября')));
const download=readFileSync('dist/download/index.html','utf8');
assert(download.includes(release.url));
assert(download.includes(release.version));
for(const route of routes){
  const file=route==='/404.html'?'dist/404.html':`dist${route}index.html`;
  const page=readFileSync(file,'utf8');
  assert(!page.includes('PDF'),'Do not advertise an unavailable export format');
  assert(page.includes('type="image/png"'));
  assert(page.includes('og:title'));
}
console.log('OK: direct installer, consistent demo, real export formats, preview/public metadata, sitemap and 404.');
