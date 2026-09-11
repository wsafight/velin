import {createReadStream} from 'node:fs';
import {stat} from 'node:fs/promises';
import {createServer} from 'node:http';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {deployment} from './deployment.mjs';

const dist = fileURLToPath(new URL('../dist', import.meta.url));
const {base} = deployment();
const port = Number(process.env.PORT || process.argv[2] || 4321);
const types = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.md': 'text/markdown; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.wasm': 'application/wasm',
  '.xml': 'application/xml; charset=utf-8',
};

function stripBase(pathname) {
  if (base === '/') return pathname;
  const prefix = base.replace(/\/$/, '');
  if (pathname === prefix || pathname === `${prefix}/`) return '/';
  if (pathname.startsWith(`${prefix}/`)) return pathname.slice(prefix.length);
  return null;
}

createServer(async (request, response) => {
  const url = new URL(request.url || '/', `http://127.0.0.1:${port}`);
  const pathname = stripBase(decodeURIComponent(url.pathname));
  if (pathname === null) {
    response.writeHead(404).end('Not found');
    return;
  }
  const relative = pathname.replace(/^\//, '');
  const candidates = [
    path.join(dist, relative),
    path.join(dist, relative, 'index.html'),
    path.join(dist, `${relative.replace(/\/$/, '')}.html`),
  ];
  for (const file of candidates) {
    try {
      if (!(await stat(file)).isFile()) continue;
      response.writeHead(200, {'Content-Type': types[path.extname(file)] || 'application/octet-stream'});
      createReadStream(file).pipe(response);
      return;
    } catch {
      // Try the next static-file candidate.
    }
  }
  response.writeHead(404, {'Content-Type': 'text/html; charset=utf-8'});
  createReadStream(path.join(dist, '404.html')).pipe(response);
}).listen(port, '127.0.0.1', () => {
  console.log(`Serving ${dist} at http://127.0.0.1:${port}${base}`);
});
