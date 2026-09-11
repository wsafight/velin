import {readFile, readdir, stat} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {load} from 'cheerio';
import {deployment} from './deployment.mjs';

const dist = fileURLToPath(new URL('../dist', import.meta.url));
const {base} = deployment();
const files = [];
const errors = [];
const idCache = new Map();

async function walk(directory) {
  for (const entry of await readdir(directory, {withFileTypes: true})) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) await walk(target);
    else if (entry.name.endsWith('.html')) files.push(target);
  }
}

async function fileExists(file) {
  try {
    return (await stat(file)).isFile();
  } catch {
    return false;
  }
}

async function resolveFile(pathname, from) {
  const relativeUrl = pathname.startsWith('/')
    ? pathname.slice(base === '/' ? 1 : base.length)
    : pathname;
  const absolute = pathname.startsWith('/')
    ? path.join(dist, relativeUrl)
    : path.resolve(path.dirname(from), relativeUrl);
  for (const candidate of [absolute, `${absolute}.html`, path.join(absolute, 'index.html')]) {
    if (await fileExists(candidate)) return candidate;
  }
  return null;
}

async function idsIn(file) {
  if (!idCache.has(file)) {
    const html = await readFile(file, 'utf8');
    const $ = load(html);
    idCache.set(file, new Set([...$('[id]')].map(node => $(node).attr('id')).filter(Boolean)));
  }
  return idCache.get(file);
}

await walk(dist);
for (const file of files) {
  const html = await readFile(file, 'utf8');
  const $ = load(html);
  const localIds = new Set([...$('[id]')].map(node => $(node).attr('id')).filter(Boolean));
  for (const node of [...$('a[href], img[src], link[href]:not([rel="canonical"]), script[src]')]) {
    const url = $(node).attr('href') || $(node).attr('src') || '';
    if (!url || /^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(url) || url.startsWith('data:')) continue;
    const [withoutHash, hash] = url.split('#');
    const pathname = withoutHash.split('?')[0];
    if (!pathname) {
      if (hash && !localIds.has(decodeURIComponent(hash))) errors.push(`${path.relative(dist, file)} -> ${url}`);
      continue;
    }
    const target = await resolveFile(pathname, file);
    if (!target) {
      errors.push(`${path.relative(dist, file)} -> ${url}`);
      continue;
    }
    if (hash && target.endsWith('.html') && !(await idsIn(target)).has(decodeURIComponent(hash))) {
      errors.push(`${path.relative(dist, file)} -> ${url}`);
    }
  }
}

if (errors.length) {
  console.error(`Broken links (${errors.length}):\n${errors.join('\n')}`);
  process.exit(1);
}
console.log(`Checked ${files.length} HTML files, no broken internal links.`);
