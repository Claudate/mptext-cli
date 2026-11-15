#!/usr/bin/env python3
"""清洗语料里的站点样板、HTML 残留与重复条目。

抓取只做了「去掉导航容器」这一层，站点模板仍留在正文里（张鑫旭每篇都带
「阅读 N 次」与版权声明，Simon 每篇带 Mastodon 关注尾注）。这些行对训练
是纯噪声，会让模型学到站点腔而不是作者文风。

原地改写，运行前请确认 output/authors 已有备份或可重新抓取。
"""
from __future__ import annotations

import argparse
import hashlib
import re
from collections import defaultdict
from pathlib import Path

# 整行删除的站点模板（按作者限定，避免误删正文里的正常句子）
LINE_RULES = {
    "张鑫旭": [
        r"^这篇文章发布于 .*$",
        r"^by \[zhangxinxu\].*$",
        r"^本文(?:欢迎|可).*转载.*$",
        r"^\s*$",
    ],
    "Simon Willison": [
        r"^Posted \[.*?Mastodon.*$",
        r"^Posted \[.*?@simon\.coop.*$",
        r"^\s*Tags: .*$",
    ],
    "Maria Popova": [
        r"^Please support The Marginalian.*$",
        r"^If you enjoyed this .*$",
    ],
}

# 全文范围的正则清理（不区分作者）
GLOBAL_RULES = [
    (re.compile(r"阅读 \d+ 次[，,]? ?今日 ?\d+ ?次"), ""),
    (re.compile(r"^\s*\d+ 条评论\s*$", re.M), ""),
    (re.compile(r"\n{3,}"), "\n\n\n"),
]

TAG_RE = re.compile(r"</?[a-zA-Z][^>]{0,200}>")
ENT_RE = re.compile(r"&(nbsp|amp|lt|gt|quot|#\d{1,5}|#x[0-9a-fA-F]{1,4});")
ENT_MAP = {"nbsp": " ", "amp": "&", "lt": "<", "gt": ">", "quot": '"'}


def clean_body(body: str, author: str) -> str:
    # HTML 残留：正文里的内联标签直接剥掉，实体还原
    if TAG_RE.search(body):
        body = TAG_RE.sub("", body)
    body = ENT_RE.sub(lambda m: ENT_MAP.get(m.group(1), " "), body)

    for pat in LINE_RULES.get(author, []):
        body = re.sub(pat, "", body, flags=re.M)
    for rx, rep in GLOBAL_RULES:
        body = rx.sub(rep, body)
    return body.strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root", nargs="?", default="output/authors")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    root = Path(args.root)
    total_changed = total_dropped = 0

    for d in sorted(p for p in root.iterdir() if p.is_dir()):
        files = sorted(d.glob("[0-9][0-9][0-9][0-9][0-9]_*.md"))
        if not files:
            continue

        changed = 0
        # 同作者内去重：标题 + 正文指纹都相同才算重复
        seen: dict[str, Path] = {}
        for f in files:
            text = f.read_text(encoding="utf-8", errors="replace")
            head, sep, body = text.partition("\n---\n")
            if not sep:
                continue
            new_body = clean_body(body, d.name)
            if new_body != body.strip():
                changed += 1
            title = text.splitlines()[0].lstrip("# ").strip()
            fp = hashlib.md5((title + new_body[:2000]).encode()).hexdigest()
            if fp in seen:
                if not args.dry_run:
                    f.unlink()
                total_dropped += 1
                print(f"  重复移除 {f.name}（与 {seen[fp].name}）")
                continue
            seen[fp] = f
            if not args.dry_run:
                f.write_text(head + sep + "\n" + new_body + "\n", encoding="utf-8")

        total_changed += changed
        print(f"{d.name:16s} {len(files):>5} 篇，改写 {changed:>5}，去重 {len(files)-len(seen):>3}")

    word = "将改写" if args.dry_run else "已改写"
    print(f"\n{word} {total_changed} 篇，移除重复 {total_dropped} 篇")


if __name__ == "__main__":
    main()
