import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {visit} from 'unist-util-visit';
import {pages} from './catalog.mjs';
import {deployment} from './deployment.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url)).replace(/[/\\]$/, '');

function docTarget(target) {
  for (const page of pages) {
    for (const lang of ['en', 'zh']) {
      if (target === page.sources[lang]) return {page, lang};
    }
  }
  return undefined;
}

export function resolveLink(url, file) {
  if (/^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(url)) return url;
  const [relative, hash = ''] = url.split('#');
  const target = path.relative(root, path.resolve(path.dirname(file), decodeURI(relative)));
  const base = deployment().base.replace(/\/$/, '');
  const doc = docTarget(target);
  if (doc) {
    const query = doc.lang === 'zh' ? '?lang=zh' : '?lang=en';
    return `${base}/${doc.page.slug}/${query}${hash ? `#${hash}` : ''}`;
  }
  if (target.startsWith('..') || path.isAbsolute(target)) {
    throw new Error(`Link escapes repository: ${url}`);
  }
  const encoded = target.split(path.sep).map(encodeURIComponent).join('/');
  return `${base}/source/${encoded}${hash ? `#${hash}` : ''}`;
}

function isLanguageSwitcher(node) {
  if (node?.type !== 'paragraph' || node.children?.length !== 1 || node.children[0].type !== 'link') {
    return false;
  }
  const label = node.children[0].children?.map(child => child.value || '').join('') || '';
  return /english|简体中文/i.test(label);
}

export function docsMarkdown() {
  return (tree, file) => {
    const firstHeading = tree.children.findIndex(node => node.type === 'heading');
    if (firstHeading >= 0 && tree.children[firstHeading].depth === 1) {
      tree.children.splice(firstHeading, 1);
    }
    if (isLanguageSwitcher(tree.children[0])) {
      tree.children.shift();
    }
    visit(tree, node => {
      if (['link', 'image', 'definition'].includes(node.type)) {
        node.url = resolveLink(node.url, file.path);
      }
    });
  };
}
