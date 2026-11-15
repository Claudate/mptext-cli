#!/usr/bin/env python3
"""验收：统计每位作者的篇数、正文长度分布，并抽查首尾文件是否像正文。"""
import re
import statistics
import sys
from pathlib import Path

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else "output/authors")
IMG = re.compile(r"!\[[^\]]*\]\([^)]*\)")
LINK = re.compile(r"\[[^\]]*\]\([^)]*\)")
JUNK = re.compile(r"[\s\-*_>`#|]+")


def text_len(body: str) -> int:
    s = LINK.sub("", IMG.sub("", body))
    return len(JUNK.sub("", s))


for d in sorted(p for p in ROOT.iterdir() if p.is_dir() and not p.name.startswith("_")):
    files = sorted(d.glob("[0-9][0-9][0-9][0-9][0-9]_*.md"))
    if not files:
        continue
    lens, titles, no_title = [], [], 0
    for f in files:
        content = f.read_text(encoding="utf-8", errors="replace")
        parts = content.split("\n---\n", 1)
        body = parts[-1]
        lens.append(text_len(body))
        first = content.splitlines()[0].lstrip("# ").strip()
        titles.append(first)
        if not first or first == "untitled":
            no_title += 1
    print(f"\n=== {d.name}：{len(files)} 篇")
    print(f"  正文字数 中位 {int(statistics.median(lens))}  最短 {min(lens)}  最长 {max(lens)}")
    print(f"  <200 字: {sum(1 for x in lens if x < 200)}   <500 字: {sum(1 for x in lens if x < 500)}   缺标题: {no_title}")
    print(f"  首篇标题: {titles[0][:70]}")
    print(f"  末篇标题: {titles[-1][:70]}")
