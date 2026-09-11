import {expect, test} from '@playwright/test';
import {deployment} from '../scripts/deployment.mjs';

const {base} = deployment();
const home = base;
const docs = `${base}docs/`;
const language = `${base}docs/language/`;
const embedding = `${base}docs/embedding/`;
const tooling = `${base}docs/tooling/`;
const architecture = `${base}docs/architecture/`;
const faq = `${base}docs/faq/`;
const playgroundGuide = `${base}docs/playground/`;
const cheatsheet = `${base}docs/cheatsheet/`;
const values = `${base}docs/values/`;
const examples = `${base}docs/examples/`;
const host = `${base}docs/host/`;
const wasm = `${base}docs/wasm/`;
const checking = `${base}docs/checking/`;
const limits = `${base}docs/limits/`;
const playground = `${base}playground/`;

test('homepage presents Velin and links to the repository', async ({page}) => {
  await page.goto(home);
  await expect(page.locator('html')).toHaveAttribute('data-lang', 'en');
  await expect(page.getByRole('heading', {level: 1})).toContainText('Write the rules.');
  await expect(page.getByText('Let the host shape reality.')).toBeVisible();
  await expect(page.locator('.hero')).not.toContainText('DIAGNOSTICS');
  await expect(page.getByRole('heading', {name: 'Start with a script you can check'})).toBeVisible();
  await expect(page.getByRole('link', {name: 'GitHub'}).first()).toHaveAttribute('href', 'https://github.com/wsafight/velin');
});

test('primary navigation opens the docs', async ({page}, testInfo) => {
  await page.goto(home);
  if (testInfo.project.name === 'mobile') {
    await page.getByRole('button', {name: 'Open menu'}).click();
    await page.getByRole('navigation', {name: 'Mobile navigation'}).getByRole('link', {name: 'Docs'}).click();
  } else {
    await page.getByRole('navigation', {name: 'Primary navigation'}).getByRole('link', {name: 'Docs'}).click();
  }
  await expect(page).toHaveURL(docs);
  await expect(page.getByRole('heading', {level: 1, name: 'Quick start'})).toBeVisible();
});

test('primary navigation exposes the detailed guides', async ({page}, testInfo) => {
  await page.goto(home);
  const navigation = testInfo.project.name === 'mobile'
    ? page.getByRole('navigation', {name: 'Mobile navigation'})
    : page.getByRole('navigation', {name: 'Primary navigation'});
  if (testInfo.project.name === 'mobile') {
    await page.getByRole('button', {name: 'Open menu'}).click();
  }
  await expect(navigation.getByRole('link', {name: 'Language'})).toHaveAttribute('href', language);
  await expect(navigation.getByRole('link', {name: 'Embedding'})).toHaveAttribute('href', embedding);
  await expect(navigation.getByRole('link', {name: 'Tooling'})).toHaveAttribute('href', tooling);
  await expect(navigation.getByRole('link', {name: 'Playground'})).toHaveAttribute('href', playground);
});

test('language selection persists and updates metadata', async ({page}) => {
  await page.goto(home);
  await page.getByRole('button', {name: 'Switch to Chinese'}).click();
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await expect(page.getByText('让脚本只负责规则')).toBeVisible();
  await expect(page).toHaveTitle('Velin - 可嵌入的确定性脚本语言');
  await page.reload();
  await expect(page.getByText('让脚本只负责规则')).toBeVisible();
  await page.getByRole('button', {name: 'Switch to English'}).click();
  await expect(page.locator('html')).toHaveAttribute('lang', 'en');
});

test('all detailed documentation routes render complete Markdown bodies', async ({page}) => {
  for (const entry of [
    {url: docs, heading: 'Quick start', section: 'Design principles'},
    {url: faq, heading: 'FAQ', section: 'What Velin is for'},
    {url: playgroundGuide, heading: 'Playground', section: 'Open the site Playground'},
    {url: language, heading: 'Language reference', section: 'File structure'},
    {url: cheatsheet, heading: 'Syntax cheat sheet', section: 'Statements'},
    {url: values, heading: 'Values and collections', section: 'The five value types'},
    {url: examples, heading: 'Examples and recipes', section: 'Run the included samples'},
    {url: embedding, heading: 'Embed in Rust', section: 'Compile and check'},
    {url: host, heading: 'Host protocol', section: 'The yield and resume cycle'},
    {url: wasm, heading: 'WebAssembly', section: 'check and run'},
    {url: tooling, heading: 'CLI, editor, and web', section: 'Command-line interface'},
    {url: architecture, heading: 'Architecture and boundaries', section: '1. Goals'},
    {url: checking, heading: 'Static checking', section: 'What the checker proves'},
    {url: limits, heading: 'Resource limits', section: 'Source and expressions'},
  ]) {
    await page.goto(entry.url);
    await expect(page.getByRole('heading', {level: 1, name: entry.heading})).toBeVisible();
    await expect(page.getByRole('heading', {level: 2, name: entry.section})).toBeVisible();
  }
});

test('documentation switches complete Markdown bodies', async ({page}) => {
  await page.goto(`${architecture}?lang=en`);
  await expect(page.getByRole('heading', {level: 2, name: '1. Goals'})).toBeVisible();
  await page.getByRole('button', {name: 'Switch to Chinese'}).click();
  await expect(page.getByRole('heading', {level: 2, name: '1. 目标'})).toBeVisible();
  await expect(page.getByText('本文只描述 Velin 自身的设计、边界与稳定性约束。')).toBeVisible();
});

test('docs sidebar, table of contents, and source links work', async ({page}, testInfo) => {
  await page.goto(docs);
  await expect(page.getByRole('navigation', {name: 'Documentation'}).getByRole('link', {name: 'FAQ'})).toBeVisible();
  await expect(page.getByRole('navigation', {name: 'Documentation'}).getByRole('link', {name: 'Examples and recipes'})).toBeVisible();
  await expect(page.getByRole('navigation', {name: 'Documentation'}).getByRole('link', {name: 'Host protocol'})).toBeVisible();
  await expect(page.getByRole('navigation', {name: 'Documentation'}).getByRole('link', {name: 'Architecture and boundaries'})).toBeVisible();
  await expect(page.getByRole('link', {name: 'Download source'})).toHaveAttribute('href', `${base}source/docs/QUICKSTART.md`);
  if (testInfo.project.name === 'desktop') {
    const toc = page.getByRole('navigation', {name: 'On this page'});
    await expect(toc.getByRole('link', {name: 'Design principles'})).toBeVisible();
  }
});

test('theme toggle changes and persists the selected theme', async ({page}) => {
  await page.addInitScript(() => {
    if (!localStorage.getItem('velin-theme')) localStorage.setItem('velin-theme', 'dark');
  });
  await page.goto(docs);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  const initialBg = await page.locator('body').evaluate(node => getComputedStyle(node).backgroundColor);
  await page.getByRole('button', {name: 'Toggle theme'}).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  const changedBg = await page.locator('body').evaluate(node => getComputedStyle(node).backgroundColor);
  expect(changedBg).not.toBe(initialBg);
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

test('code samples can be copied', async ({page, context}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto(docs);
  const button = page.locator('.copy-code').first();
  await expect(button).toHaveAccessibleName('Copy');
  await button.click();
  await expect(button).toHaveAccessibleName('Copied');
});

test('Wasm Playground checks and runs the sample', async ({page}) => {
  await page.goto(playground);
  await expect(page.getByRole('heading', {level: 1, name: 'Velin Playground'})).toBeVisible();
  const run = page.getByRole('button', {name: 'Run'});
  await expect(run).toBeEnabled({timeout: 30000});
  await expect(page.getByText('No problems.')).toBeVisible();
  await run.click();
  await expect(page.locator('#output')).toContainText('You feel restored.');
  await expect(page.locator('#output')).toContainText('40');
});

test('Playground links to every main documentation area', async ({page}, testInfo) => {
  await page.goto(playground);
  await expect(page.getByRole('link', {name: 'Velin home'})).toHaveAttribute('href', '../');
  if (testInfo.project.name === 'mobile') {
    await page.getByRole('button', {name: 'Open menu'}).click();
  }
  const navigation = testInfo.project.name === 'mobile'
    ? page.getByRole('navigation', {name: 'Mobile navigation'})
    : page.getByRole('navigation', {name: 'Product navigation'});
  await expect(navigation.getByRole('link', {name: 'Language'})).toHaveAttribute('href', '../docs/language/');
  await expect(navigation.getByRole('link', {name: 'Embedding'})).toHaveAttribute('href', '../docs/embedding/');
  await expect(navigation.getByRole('link', {name: 'Tooling'})).toHaveAttribute('href', '../docs/tooling/');
  await expect(navigation.getByRole('link', {name: 'Docs'})).toHaveAttribute('href', '../docs/');
  await expect(navigation.getByRole('link', {name: 'Playground'})).toHaveAttribute('href', './');
});

test('homepage and Playground do not overflow the viewport', async ({page}) => {
  for (const url of [home, playground]) {
    await page.goto(url);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
  }
});

test('Playground follows the stored site language', async ({page}) => {
  await page.goto(`${home}?lang=zh`);
  await page.goto(playground);
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await expect(page.getByRole('heading', {level: 2, name: '诊断'})).toBeVisible();
});
