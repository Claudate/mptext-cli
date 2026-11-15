import { chromium } from 'playwright';
const browser = await chromium.launch({ headless: true, channel: 'chrome' });
const ctx = await browser.newContext({
  userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  locale: 'zh-CN', viewport: { width: 1280, height: 900 },
});
const page = await ctx.newPage();
const url = process.argv[2];
await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => null);
await page.waitForTimeout(4000);
const html = await page.content();
console.log('title:', JSON.stringify(await page.title()), 'len:', html.length);
console.log('signals:', ['环境异常','验证','js_content','rich_media_title','album','合集'].filter(s => html.includes(s)));
const info = await page.evaluate(() => {
  const pick = s => (document.querySelector(s) || {}).innerText || '';
  return {
    title: pick('#activity-name') || pick('.rich_media_title') || document.title,
    nickname: pick('#js_name') || pick('.rich_media_meta_nickname'),
    contentLen: (pick('#js_content') || pick('.rich_media_content')).length,
    preview: (pick('#js_content') || '').slice(0, 150),
    albums: [...document.querySelectorAll('a[href*="appmsgalbum"], [data-album-id], mpprofile, mp-common-album')]
      .map(a => (a.outerHTML || '').slice(0, 160)),
  };
});
console.log(JSON.stringify(info, null, 1).slice(0, 2000));
await page.screenshot({ path: '/tmp/article_shot.png', fullPage: false });
await browser.close();
