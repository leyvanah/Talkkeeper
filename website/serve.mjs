import {createServer} from 'node:http';
import {readFileSync,statSync} from 'node:fs';
import {resolve,sep,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('./dist/',import.meta.url)).replace(/[\\/]$/,'');
const port=Number(process.env.PORT||4173);
const types={'.html':'text/html; charset=utf-8','.css':'text/css; charset=utf-8','.js':'text/javascript; charset=utf-8','.json':'application/json','.xml':'application/xml; charset=utf-8','.txt':'text/plain; charset=utf-8','.png':'image/png','.svg':'image/svg+xml','.woff2':'font/woff2'};
createServer((req,res)=>{
  try {
    const {basePath}=JSON.parse(readFileSync(resolve(root,'site-config.json'),'utf8'));
    let pathname=decodeURIComponent(new URL(req.url,'http://localhost').pathname);
    if(basePath&&pathname==='/'){res.writeHead(302,{Location:basePath+'/'}).end();return;}
    if(basePath&&pathname===basePath){res.writeHead(301,{Location:basePath+'/'}).end();return;}
    if(!pathname.startsWith(basePath+'/'))throw new Error('Outside site');
    pathname=pathname.slice(basePath.length);
    let file=resolve(root,'.'+pathname);
    if(file!==root&&!file.startsWith(root+sep)){res.writeHead(403).end();return;}
    if(statSync(file).isDirectory())file=resolve(file,'index.html');
    res.writeHead(200,{'Content-Type':types[extname(file)]||'application/octet-stream'}).end(readFileSync(file));
  }catch{res.writeHead(404,{'Content-Type':'text/html; charset=utf-8'}).end(readFileSync(resolve(root,'404.html')));}
}).listen(port,'127.0.0.1',()=>console.log('Local URL: http://127.0.0.1:'+port+'/'));
