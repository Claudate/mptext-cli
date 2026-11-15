import { chromium } from 'playwright';
import fs from 'fs';
import os from 'os';

const sess = JSON.parse(fs.readFileSync(os.homedir() + '/.config/mptext/session.json', 'utf8'));
const token = sess.token;
let ck = sess.cookies;
const pairs = Array.isArray(ck) ? ck : (typeof ck === 'string'
  ? ck.split(';').filter(s => s.includes('=')).map(s => { const i = s.indexOf('='); return { name: s.slice(0, i).trim(), value: s.slice(i + 1).trim() }; })
  : Object.entries(ck).map(([name, value]) => ({ name, value })));

const browser = await chromium.launch({ headless: true, channel: 'chrome' });
const ctx = await browser.newContext({
  userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
  locale: 'zh-CN',
  viewport: { width: 1440, height: 900 },
});
await ctx.addCookies(pairs.map(c => ({ ...c, domain: '.weixin.qq.com', path: '/' })));

const page = await ctx.newPage();
const BIZ = process.argv[2] || 'Mzg2OTg5NDg3Mg==';

// 1) 先打卡后台首页，建立正常导航上下文
const homeUrl = `https://mp.weixin.qq.com/cgi-bin/home?t=home/index&lang=zh_CN&token=${token}`;
const r1 = await page.goto(homeUrl, { waitUntil: 'domcontentloaded', timeout: 30000 }).catch(e => null);
console.log('HOME status:', r1 ? r1.status() : 'ERR', 'title:', await page.title().catch(() => '?'));

// 2) 再导航到"已发表"页面
const pubUrl = `https://mp.weixin.qq.com/cgi-bin/appmsg?t=media/appmsg_list_v2&action=list_ex&begin=0&count=5&fakeid=${encodeURIComponent(BIZ)}&type=9&token=${token}&lang=zh_CN`;
const r2 = await page.goto(pubUrl, { waitUntil: 'networkidle', timeout: 30000 }).catch(e => null);
console.log('PUB status:', r2 ? r2.status() : 'ERR', 'url:', page.url().slice(0, 120));

const body = await page.evaluate(() => document.body ? document.body.innerText.slice(0, 800) : '(no body)');
console.log('--- body ---');
console.log(body.replace(/\s+/g, ' ').slice(0, 800));
const html = await page.content();
console.log('--- signals ---');
for (const m of ['200013', 'app_msg_list', 'freq control', '总用户数', '发表记录']) {
  if (html.includes(m)) console.log('  含:', m);
}
await browser.close();
