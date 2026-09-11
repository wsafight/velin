export function deployment() {
  const repository = process.env.GITHUB_REPOSITORY;
  const [owner, name] = repository?.split('/') || [];
  const customSite = process.env.DOCS_SITE || '';
  const customBase = process.env.DOCS_BASE || '';
  const site = customSite || (owner ? `https://${owner}.github.io` : 'http://localhost:4321');
  const path = customBase || (customSite || !name || name === `${owner}.github.io` ? '/' : `/${name}/`);
  const segments = path.split('/').filter(Boolean);
  const base = segments.length ? `/${segments.join('/')}/` : '/';
  return {site, base};
}
