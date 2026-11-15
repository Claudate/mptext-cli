#!/usr/bin/env python3
"""按作者抓取全量文章并落盘为 Markdown。

本文件是本次「四作者文章库」的采集器：给定作者 → 按该站的列表通道枚举文章 →
抓正文 → 清掉导航/分享/评论 → 转 Markdown 落盘。

几个不显然的取舍：
- 全部走 curl 子进程而不是 requests：本机 python 直连外部站点会在 TLS 层被掐断，
  curl 走系统代理正常。
- 断点续传以文件头里的「原文」URL 为键，不以文件名序号为键：列表通道重排序号
  （Blogger 的 feed 游标会变）时不会重复下载。
- 只要文字文章：去掉 Markdown 图片/链接后剩余文字不足 200 字的记为图片帖，不收。

四个源：张鑫旭（WordPress REST）、和菜头（Blogger Atom feed）、
Simon Willison（年度归档页）、Maria Popova（WordPress REST）。
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import html
import json
import re
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path

import html2text
from bs4 import BeautifulSoup

UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/131.0 Safari/537.36")

# 正文里必须剔除的容器（导航、分享、评论、订阅、相关阅读等）。
# 注意：只按语义标签选，不要写 header/footer/nav 的类名，Blogger 的 .post-header 在正文内。
DROP_SELECTORS = [
    "script", "style", "noscript", "iframe", "form",
    "nav", "body > header", "body > footer",
    ".share", ".sharing", ".social", ".comments", ".comment", "#comments",
    ".sidebar", ".widget", ".related", ".related-posts", ".newsletter",
    ".subscribe", ".subscription", ".ads", ".advert", ".advertisement",
    ".post-navigation", ".pagination", ".breadcrumb", ".author-box",
    ".sociable", ".jp-relatedposts", ".sharedaddy", ".wpcnt",
    ".post-footer", ".post-share-buttons", ".post-timestamp", ".comments-link",
]


class Http:
    """带限速与重试的 curl 封装（python 直连在本机会被 TLS 层掐断，curl 正常）。"""

    def __init__(self, delay: float = 0.35, timeout: int = 40):
        self.delay = delay
        self.timeout = timeout
        self.last = 0.0

    def get(self, url: str, tries: int = 4) -> str:
        for attempt in range(tries):
            gap = time.time() - self.last
            if gap < self.delay:
                time.sleep(self.delay - gap)
            proc = subprocess.run(
                ["curl", "-sL", "--compressed", "-m", str(self.timeout),
                 "-A", UA, "-H", "Accept-Language: zh-CN,zh;q=0.9,en;q=0.8", url],
                capture_output=True, text=True, errors="replace")
            self.last = time.time()
            body = proc.stdout or ""
            if body and len(body) > 500:
                return body
            # 站点偶发 5xx / 限流，退避后重试；仍失败由调用方记为失败
            time.sleep(2.0 * (attempt + 1))
        return ""


@dataclass
class Item:
    url: str
    title: str
    published: str


# ---------------------------------------------------------------- 正文抽取

def strip_boilerplate(soup: BeautifulSoup) -> BeautifulSoup:
    for sel in DROP_SELECTORS:
        for node in soup.select(sel):
            node.decompose()
    return soup


def to_markdown(node) -> str:
    h = html2text.HTML2Text(baseurl="")
    h.ignore_links = False
    h.ignore_images = False
    h.body_width = 0
    h.ignore_emphasis = False
    text = h.handle(str(node))
    text = re.sub(r"\n{4,}", "\n\n\n", text)
    return text.strip()


def pick_container(soup: BeautifulSoup, selectors: list[str]):
    """按候选选择器挑正文容器；都失败就用 <article>/<body> 兜底。"""
    for sel in selectors:
        node = soup.select_one(sel)
        if node and len(node.get_text(strip=True)) > 200:
            return node
    article = soup.find("article")
    if article and len(article.get_text(strip=True)) > 200:
        return article
    return soup.body or soup


# ---------------------------------------------------------------- 各源实现

class Zhengxinxu:
    """张鑫旭（中文科技 / 前端）：WordPress，REST API 可取全量列表。"""
    name = "张鑫旭"
    base = "https://www.zhangxinxu.com/wordpress"
    total = 1008

    def __init__(self, http: Http):
        self.http = http

    def list_items(self, limit: int | None = None):
        per_page = 100
        page = 1
        got = 0
        while True:
            url = (f"{self.base}/wp-json/wp/v2/posts?per_page={per_page}"
                   f"&page={page}&_fields=link,title,date")
            raw = self.http.get(url)
            if not raw:
                break
            try:
                batch = json.loads(raw)
            except json.JSONDecodeError:
                break
            if not isinstance(batch, list) or not batch:
                break
            for p in batch:
                yield Item(p["link"], html.unescape(p["title"]["rendered"]), p["date"])
                got += 1
                if limit and got >= limit:
                    return
            page += 1

    def extract(self, url: str):
        raw = self.http.get(url)
        if not raw:
            return None
        soup = BeautifulSoup(raw, "lxml")
        # 站点标题带「« 张鑫旭-鑫空间-鑫生活」后缀，取正文首个 h1 更准
        h1 = soup.select_one("#content h1, .entry-title, h1")
        title = h1.get_text(strip=True) if h1 else ""
        date = ""
        meta = soup.find("meta", {"property": "article:published_time"})
        if meta:
            date = meta.get("content", "")
        node = pick_container(soup, ["#content", ".entry-content", "div.content", "#post"])
        strip_boilerplate(node)
        body = to_markdown(node)
        return title, date, body


class Hecaitou:
    """和菜头（中文人文 / 槽边往事）：Blogger Atom feed 可分页取全量。"""
    name = "和菜头"
    feed = "https://www.hecaitou.com/feeds/posts/default"
    total = 1293

    def __init__(self, http: Http):
        self.http = http

    def list_items(self, limit: int | None = None):
        # Blogger 会把 max-results 压到实际返回条数（常见 88–127），
        # 所以游标必须按本次实际拿到的条数推进，否则会漏文。
        want = 150
        start = 1
        got = 0
        while True:
            url = (f"{self.feed}?alt=json&max-results={want}&start-index={start}")
            raw = self.http.get(url)
            if not raw:
                break
            try:
                feed = json.loads(raw)["feed"]
            except (json.JSONDecodeError, KeyError):
                break
            entries = feed.get("entry", [])
            if not entries:
                break
            for e in entries:
                link = ""
                for l in e.get("link", []):
                    if l.get("rel") == "alternate":
                        link = l["href"]
                        break
                if not link:
                    continue
                # Blogger 的部分标题会把空格带进 alternate 链接，先做一次 URL 编码校正
                link = clean_url(link)
                yield Item(link, e["title"]["$t"], e.get("published", {}).get("$t", ""))
                got += 1
                if limit and got >= limit:
                    return
            start += len(entries)

    def extract(self, url: str):
        raw = self.http.get(url)
        if not raw:
            return None
        soup = BeautifulSoup(raw, "lxml")
        title_node = soup.select_one("h3.post-title, h1.post-title, .post-title")
        title = title_node.get_text(strip=True) if title_node else ""
        date = ""
        ts = soup.select_one("h2.date-header, .post-timestamp")
        if ts:
            date = ts.get_text(strip=True)
        # Blogger 的空格用 <br> 分隔段落，先还原成空行再转 Markdown
        node = pick_container(soup, [".post-body", ".entry-content", "#post-body"])
        strip_boilerplate(node)
        for br in node.find_all("br"):
            br.replace_with("\n\n")
        return title, date, to_markdown(node)


class SimonWillison:
    """Simon Willison（英文科技）：/YYYY/ 年度归档列出当年全部条目。"""
    name = "Simon Willison"
    base = "https://simonwillison.net"

    def __init__(self, http: Http):
        self.http = http

    def list_items(self, limit: int | None = None):
        got = 0
        seen = set()
        for year in range(2026, 2001, -1):
            raw = self.http.get(f"{self.base}/{year}/")
            if not raw:
                continue
            for m in re.finditer(r'href="(/%d/[A-Za-z]{3}/\d{1,2}/[^"/]+/)"' % year, raw):
                path = m.group(1)
                if path in seen:
                    continue
                seen.add(path)
                yield Item(self.base + path, "", "")
                got += 1
                if limit and got >= limit:
                    return

    def extract(self, url: str):
        raw = self.http.get(url)
        if not raw:
            return None
        soup = BeautifulSoup(raw, "lxml")
        date = ""
        t = soup.select_one(".entry-date, time")
        if t:
            date = t.get_text(strip=True) or t.get("datetime", "")
        # 站名在 <h1> 里，真正的文章标题是 .entry 内的 h2
        title_node = soup.select_one(".entry h2, .entry-title")
        title = title_node.get_text(strip=True) if title_node else ""
        node = pick_container(soup, [".entry", ".entry-content", "#primary .content"])
        if title_node and title_node.parent is not None:
            title_node.extract()
        strip_boilerplate(node)
        return title, date, to_markdown(node)


class Marginalian:
    """Maria Popova（英文人文 / The Marginalian）：WordPress，REST API 全量 6701 篇。"""
    name = "Maria Popova"
    base = "https://www.themarginalian.org"

    def __init__(self, http: Http):
        self.http = http

    def list_items(self, limit: int | None = None):
        per_page = 100
        page = 1
        got = 0
        empty_streak = 0
        while True:
            url = (f"{self.base}/wp-json/wp/v2/posts?per_page={per_page}"
                   f"&page={page}&_fields=link,title,date")
            # 本机出站偶发抖动，单次失败不代表已到末页；连续两页都空才收手
            raw = self.http.get(url, tries=6)
            if not raw:
                empty_streak += 1
                if empty_streak >= 2:
                    break
                page += 1
                continue
            empty_streak = 0
            try:
                batch = json.loads(raw)
            except json.JSONDecodeError:
                break
            if not isinstance(batch, list) or not batch:
                break
            for p in batch:
                yield Item(p["link"], html.unescape(p["title"]["rendered"]), p["date"])
                got += 1
                if limit and got >= limit:
                    return
            page += 1

    def extract(self, url: str):
        raw = self.http.get(url)
        if not raw:
            return None
        soup = BeautifulSoup(raw, "lxml")
        # Marginalian 的文章标题在 h1.entry-title，站点的「Archives」之类是导航 h1
        title_node = soup.select_one("h1.entry-title")
        title = title_node.get_text(strip=True) if title_node else ""
        date = ""
        for prop in ("article:published_time", "article:modified_time"):
            meta = soup.find("meta", {"property": prop})
            if meta and meta.get("content"):
                date = meta["content"]
                break
        if not date:
            t = soup.select_one("time.entry-date, time")
            if t:
                date = t.get("datetime", "") or t.get_text(strip=True)
        # 该站的正文容器类名是下划线风格 .entry_content（#content 还含站点导航）
        node = pick_container(soup, [".entry_content", ".entry-content", "#content"])
        strip_boilerplate(node)
        return title, date, to_markdown(node)


SOURCES = {
    "zhengxinxu": Zhengxinxu,
    "hecaitou": Hecaitou,
    "simon": SimonWillison,
    "marginalian": Marginalian,
}


# ---------------------------------------------------------------- 落盘

BAD_CHARS = re.compile(r'[\\/:*?"<>|\n\r\t]+')
URL_RE = re.compile(r"^- 原文：(\S+)$", re.MULTILINE)


def clean_url(u: str) -> str:
    """修复 feed 里被标题文本污染的链接：去掉路径中意外出现的空格。

    Blogger 的部分 <link rel="alternate"> 会把标题里的空格带进 href（如
    /2023/06/ details-of-history.html），直接请求会 404。这里只清理路径段。
    """
    return re.sub(r"\s+", "", u)


def safe_name(s: str, limit: int = 80) -> str:
    s = BAD_CHARS.sub("_", (s or "").strip()).strip("._ ")
    return (s[:limit] or "untitled")


def run(source_key: str, out_root: Path, limit: int | None, delay: float, workers: int = 8):
    """抓取一个作者。列表单线程取（保持稳定序号），正文按线程池并发抓。

    序号只在首次运行时确定：已存在的文件按序号前缀跳过，所以重跑不会打乱已有编号。
    """
    src = SOURCES[source_key](Http(delay=delay))
    outdir = out_root / src.name
    outdir.mkdir(parents=True, exist_ok=True)

    items = list(src.list_items(limit=limit))
    print(f"[{src.name}] 列表共 {len(items)} 条，开始并发抓取（{workers} 并）", flush=True)

    # 断点续传以「原文 URL」为键，不用文件名前缀：列表通道重排序号时不会重复下载。
    done_urls = {}
    for f in outdir.glob("[0-9][0-9][0-9][0-9][0-9]_*.md"):
        head = f.read_text(encoding="utf-8", errors="replace")[:800]
        m = URL_RE.search(head)
        if m:
            done_urls[m.group(1)] = f.name

    index_rows = []
    stats = {"written": 0, "skipped": 0, "failed": 0}
    lock = threading.Lock()
    done = 0

    def work(pair):
        nonlocal done
        i, item = pair
        prefix = f"{i:05d}"
        existing = done_urls.get(item.url)
        if existing:
            with lock:
                stats["skipped"] += 1
                index_rows.append((prefix, item.url, existing))
            return
        got = src.extract(item.url)
        if not got:
            with lock:
                stats["failed"] += 1
            print(f"[{i}] FAIL {item.url}", flush=True)
            return
        title, date, body = got
        title = title or item.title or "untitled"
        # 用户要求只要文字文章：去掉 Markdown 图片与链接后，剩余文字太短的视为图片帖，不收。
        text_only = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", body)
        text_only = re.sub(r"\[[^\]]*\]\([^)]*\)", "", text_only)
        text_only = re.sub(r"[\s\-*_>`#|]+", "", text_only)
        if len(text_only) < 200:
            with lock:
                stats["skipped"] += 1
            print(f"[{i}] IMAGE-ONLY 跳过 {item.url}", flush=True)
            return

        fname = f"{prefix}_{safe_name(title)}.md"
        # 同序号已被其他条目占用时（列表顺序变动），补上 URL 尾部避开冲突
        if (outdir / fname).exists() and done_urls.get(item.url) != fname:
            fname = f"{prefix}_{safe_name(title)}_{safe_name(item.url.rstrip('/').split('/')[-1], 24)}.md"
        head = (f"# {title}\n\n"
                f"- 作者：{src.name}\n"
                f"- 发布：{date or item.published or '未知'}\n"
                f"- 原文：{item.url}\n\n---\n\n")
        (outdir / fname).write_text(head + body + "\n", encoding="utf-8")
        with lock:
            stats["written"] += 1
            done += 1
            index_rows.append((prefix, item.url, fname))
            if done % 50 == 0:
                print(f"[{src.name}] 进行中：写 {stats['written']} 跳 {stats['skipped']} 败 {stats['failed']}", flush=True)

    with cf.ThreadPoolExecutor(max_workers=workers) as ex:
        list(ex.map(work, enumerate(items, 1)))

    index_rows.sort(key=lambda r: r[0])
    with (outdir / "_index.tsv").open("w", encoding="utf-8") as f:
        f.write("no\turl\tfile\n")
        for row in index_rows:
            f.write("\t".join(row) + "\n")

    print(f"[{src.name}] 完成：写入 {stats['written']}，跳过 {stats['skipped']}，失败 {stats['failed']} -> {outdir}", flush=True)
    return stats["written"], stats["skipped"], stats["failed"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("source", choices=list(SOURCES))
    ap.add_argument("-o", "--out", default="output/authors")
    ap.add_argument("-l", "--limit", type=int, default=None)
    ap.add_argument("-d", "--delay", type=float, default=0.35)
    ap.add_argument("-w", "--workers", type=int, default=8)
    args = ap.parse_args()
    run(args.source, Path(args.out).expanduser(), args.limit, args.delay, args.workers)


if __name__ == "__main__":
    main()
