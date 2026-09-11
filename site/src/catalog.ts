import {groups, pages} from '../scripts/catalog.mjs';

export {groups, pages};
export type DocPage = (typeof pages)[number];

export const href = (slug = '') =>
  `${import.meta.env.BASE_URL.replace(/\/$/, '')}/${slug}${slug ? '/' : ''}`;

export const groupedPages = groups.map(group => ({
  ...group,
  pages: pages.filter(page => page.group === group.id),
}));

export function neighbors(slug: string) {
  const index = pages.findIndex(page => page.slug === slug);
  return {
    previous: index > 0 ? pages[index - 1] : undefined,
    next: index >= 0 && index < pages.length - 1 ? pages[index + 1] : undefined,
  };
}

export function relatedPages(slug: string) {
  const page = pages.find(item => item.slug === slug) as (DocPage & {see?: string[]}) | undefined;
  return (page?.see ?? [])
    .map(id => pages.find(item => item.slug === id))
    .filter((item): item is DocPage => Boolean(item));
}
