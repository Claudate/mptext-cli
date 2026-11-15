import { chromium } from 'playwright';
const browser = await chromium.launch({ headless: true, channel: 'chrome' });
const ctx = await browser.newContext({
  userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.40',
  locale: 'zh-CN', viewport: { width: 430, height: 932 }, isMobile: true, hasTouch: true,
});
const page = await ctx.newPage();
const url = process.argv[2];
const resp = await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 }).catch(() => null);
console.log('status:', resp ? resp.status() : 'ERR', 'title:', await page.title());
await page.waitForTimeout(2500);
const html = await page.content();
console.log('len:', html.length);
console.log('signals:', ['环境异常','验证','album','合集','js_content'].filter(s => html.includes(s)));

const info = await page.evaluate(() => {
  const r = {};
  r.title = document.title;
  const c = document.querySelector('#js_content') || document.querySelector('.rich_media_content');
  r.contentLen = c ? c.innerText.length : 0;
  r.albums = [...document.querySelectorAll('a[href*="appmsgalbum"], [data-album-id]')]
    .map(a => ({ text: a.innerText.trim().slice(0,30), href: a.getAttribute('href') || '', id: a.getAttribute('data-album-id') || '' }));
  r.nickname = (document.querySelector('#js_name, .rich_media_meta_nickname') || {}).innerText || '';
  return r;
});
console.log(JSON.stringify(info, null, 1).slice(0, 1500));
await browser.close();
