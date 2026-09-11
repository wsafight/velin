export type Lang = 'en' | 'zh';

export const ui = {
  en: {
    skip: 'Skip to main content', home: 'Velin home', primaryNav: 'Primary navigation',
    mobileNav: 'Mobile navigation', footerNav: 'Footer navigation', docsNav: 'Documentation',
    toc: 'On this page', pager: 'Adjacent pages', theme: 'Toggle theme',
    menuOpen: 'Open menu', menuClose: 'Close menu', lang: 'Switch to Chinese',
    copy: 'Copy', copied: 'Copied', language: 'Language', embedding: 'Embedding', tooling: 'Tooling',
    docs: 'Docs', playground: 'Playground', github: 'GitHub',
  },
  zh: {
    skip: '跳到主要内容', home: 'Velin 首页', primaryNav: '主要导航',
    mobileNav: '移动端导航', footerNav: '页脚导航', docsNav: '文档目录',
    toc: '本页内容', pager: '相邻文档', theme: '切换主题',
    menuOpen: '打开菜单', menuClose: '关闭菜单', lang: 'Switch to English',
    copy: '复制', copied: '已复制', language: '语言', embedding: '嵌入', tooling: '工具链',
    docs: '文档', playground: 'Playground', github: 'GitHub',
  },
} as const;

export const githubRepo = 'https://github.com/wsafight/velin';

export const pagesMeta = {
  home: {
    title: {en: 'Velin - deterministic scripts for any host', zh: 'Velin - 可嵌入的确定性脚本语言'},
    description: {
      en: 'Velin is a resource-bounded, statically checked bytecode scripting language. Scripts own rules and control flow; the host owns every effect. The same pipeline also runs in the browser as WebAssembly.',
      zh: 'Velin 是一门资源有界、可静态检查的字节码脚本语言。脚本负责规则与控制流，宿主负责全部外部效果。同一套管线也可作为 WebAssembly 在浏览器中运行。',
    },
  },
  notFound: {
    title: {en: 'Page not found - Velin', zh: '页面不存在 - Velin'},
    description: {en: 'The requested Velin page could not be found.', zh: '请求的 Velin 页面不存在。'},
  },
};

export function langFromStorage(value: string | null): Lang {
  return value === 'zh' ? 'zh' : 'en';
}
