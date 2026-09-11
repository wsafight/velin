import {copyFile, cp, mkdir, readFile, rm, stat, writeFile} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {remark} from 'remark';
import {pages} from './catalog.mjs';
import {docsMarkdown} from './markdown.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url)).replace(/[/\\]$/, '');
const generated = path.join(root, 'site/.generated');
const publicSource = path.join(root, 'site/public/source');
const publicPlayground = path.join(root, 'site/public/playground');
const processor = remark().use(docsMarkdown);

await rm(generated, {recursive: true, force: true});
await rm(publicSource, {recursive: true, force: true});
await rm(publicPlayground, {recursive: true, force: true});

for (const page of pages) {
  for (const lang of ['en', 'zh']) {
    const source = path.join(root, page.sources[lang]);
    const original = await readFile(source, 'utf8');
    const rendered = await processor.process({path: source, value: original});
    const output = path.join(generated, lang, page.generated);
    await mkdir(path.dirname(output), {recursive: true});
    await writeFile(output, String(rendered));
    const destination = path.join(publicSource, page.sources[lang]);
    await mkdir(path.dirname(destination), {recursive: true});
    await copyFile(source, destination);
  }
}

await cp(path.join(root, 'examples'), path.join(publicSource, 'examples'), {recursive: true});

const playgroundSource = path.join(root, 'web/playground');
await mkdir(publicPlayground, {recursive: true});
for (const file of ['index.html', 'style.css', 'playground.js', 'worker.js']) {
  await copyFile(path.join(playgroundSource, file), path.join(publicPlayground, file));
}
try {
  if ((await stat(path.join(playgroundSource, 'pkg'))).isDirectory()) {
    await cp(path.join(playgroundSource, 'pkg'), path.join(publicPlayground, 'pkg'), {recursive: true});
  }
} catch {
  // Type checking does not require Wasm; the production build creates it first.
}

const {version} = JSON.parse(await readFile(path.join(root, 'site/package.json'), 'utf8'));
await writeFile(path.join(root, 'site/public/version.json'), JSON.stringify({version}));
