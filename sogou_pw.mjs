import { chromium } from 'playwright';
const browser = await chromium.launch({ headless: true, channel: 'chrome' });
const ctx = await browser.newContext({
  userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  locale: 'zh-CN', viewport: { width: 1440, height: 900 },
});
const page = await ctx.newPage();
const url = 'https://weixin.sogou.com/weixin?type=1&query=' + encodeURIComponent('手工川') + '&ie=utf8';
const resp = await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 }).catch(e => null);
console.log('status:', resp ? resp.status() : 'ERR');
console.log('title:', await page.title());
const html = await page.content();
console.log('len:', html.length, 'antispider:', html.includes('antispider'), 'captcha:', html.includes('验证码'));
// 提取账号列表
const accounts = await page.evaluate(() => {
  const out = [];
  document.querySelectorAll('ul.news-list2 li, .wx-rb').forEach(el => {
    const a = el.querySelector('a');
    out.push({ text: el.innerText.replace(/\s+/g, ' ').slice(0, 120), href: a ? a.href : '' });
  });
  return out;
});
console.log('账号条目:', accounts.length);
accounts.slice(0, 5).forEach(a => console.log('  ', a.text));
// 直接找"手工川"精确匹配的链接
const link = await page.evaluate(() => {
  for (const a of document.querySelectorAll('a')) {
    if (a.innerText.trim() === '手工川') return a.href;
  }
  return '';
});
console.log('精确链接:', link.slice(0, 200));
await browser.close();
