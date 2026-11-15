#!/usr/bin/env python3
"""清理纯图片帖，并统计每位作者的纯文字文章数（一次性脚本）。"""
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else "output/authors")
QUARANTINE = ROOT / "_image_only"

IMG = re.compile(r"!\[[^\]]*\]\([^)]*\)")
LINK = re.compile(r"\[[^\]]*\]\([^)]*\)")
JUNK = re.compile(r"[\s\-*_>`#|]+")


def text_len(body: str) -> int:
    s = IMG.sub("", body)
    s = LINK.sub("", s)
    return len(JUNK.sub("", s))


def main():
    QUARANTINE.mkdir(parents=True, exist_ok=True)
    total_kept = total_moved = 0
    for author_dir in sorted(p for p in ROOT.iterdir() if p.is_dir() and not p.name.startswith("_")):
        kept = moved = 0
        for f in sorted(author_dir.glob("[0-9][0-9][0-9][0-9][0-9]_*.md")):
            content = f.read_text(encoding="utf-8", errors="replace")
            body = content.split("\n---\n", 1)[-1]
            if text_len(body) < 200:
                dest = QUARANTINE / author_dir.name
                dest.mkdir(parents=True, exist_ok=True)
                shutil.move(str(f), str(dest / f.name))
                moved += 1
            else:
                kept += 1
        total_kept += kept
        total_moved += moved
        print(f"{author_dir.name:16s} 保留 {kept:>5}  移出图片帖 {moved:>4}")
    print(f"合计：保留 {total_kept}，移出 {total_moved}")


if __name__ == "__main__":
    main()
