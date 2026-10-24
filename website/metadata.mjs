const escape = value => value.replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;');
export function siteSettings(env=process.env) {
  const isPublic=env.SITE_PUBLIC==='1';
  let origin='';
  let basePath='';
  if(env.SITE_URL) {
    const url=new URL(env.SITE_URL);
    if(url.protocol!=='https:'||url.username||url.password||url.search||url.hash||!/^\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]*\/?$/.test(url.pathname))throw new Error('SITE_URL must be an HTTPS site URL, optionally with a repository path.');
    origin=url.origin;
    basePath=url.pathname.replace(/\/$/,'');
  }
  if(isPublic&&!origin)throw new Error('Set SITE_URL to the real domain before building a public site.');
  return {isPublic,origin,basePath};
}
// Fragment-only links stay on the current page; external URLs stay external.
export function withBase(path,settings) {
  return /^\/(?!\/)/.test(path)?settings.basePath+path:path;
}
export function prefixHTML(html,settings) {
  return html.replace(/\b(href|src)="(\/(?!\/)[^"]*)"/g,(_,attr,path)=>`${attr}="${withBase(path,settings)}"`);
}
export function prefixCSS(css,settings) {
  return css.replace(/url\((['"]?)(\/(?!\/)[^)'"\s]+)\1\)/g,(_,quote,path)=>`url(${quote}${withBase(path,settings)}${quote})`);
}
export function metadata({path,title,description,settings}) {
  const {isPublic,origin}=settings;
  const missing=path==='/404.html';
  const url=origin+withBase(path,settings);
  const image=origin+withBase('/assets/talkkeeper.png',settings);
  return `<meta name="description" content="${escape(description)}"><meta name="robots" content="${isPublic&&!missing?'index, follow':'noindex, nofollow'}">${origin&&!missing?`<link rel="canonical" href="${escape(url)}">`:''}<meta property="og:type" content="website"><meta property="og:locale" content="ru_RU"><meta property="og:site_name" content="Talkkeeper"><meta property="og:title" content="${escape(title)} — Talkkeeper"><meta property="og:description" content="${escape(description)}">${origin?`<meta property="og:url" content="${escape(url)}">`:''}<meta property="og:image" content="${escape(image)}"><meta property="og:image:alt" content="Логотип Talkkeeper: звуковая волна и перо"><meta name="twitter:card" content="summary"><meta name="twitter:title" content="${escape(title)} — Talkkeeper"><meta name="twitter:description" content="${escape(description)}"><meta name="twitter:image" content="${escape(image)}">`;
}
export function discovery(settings,routes) {
  const {isPublic,origin}=settings;
  const robots=`User-agent: *\n${isPublic?'Allow: /':'Disallow: /'}\n${isPublic?`Sitemap: ${origin}${withBase('/sitemap.xml',settings)}\n`:''}`;
  const urls=origin&&isPublic?routes.filter(path=>path!=='/404.html').map(path=>`<url><loc>${escape(origin+withBase(path,settings))}</loc></url>`).join(''):'';
  return {robots,sitemap:`<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${urls}</urlset>`};
}
