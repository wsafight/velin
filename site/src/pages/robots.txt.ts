import type {APIRoute} from 'astro';

export const GET: APIRoute = ({site}) => {
  const sitemap = site
    ? new URL(`${import.meta.env.BASE_URL}sitemap-index.xml`, site).href
    : `${import.meta.env.BASE_URL}sitemap-index.xml`;
  return new Response(`User-agent: *\nAllow: /\nSitemap: ${sitemap}\n`, {
    headers: {'Content-Type': 'text/plain; charset=utf-8'},
  });
};
