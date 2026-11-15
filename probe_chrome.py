"""Chrome 96+ 的 cookie 在 <profile>/Network/Cookies。看 mp.weixin.qq.com 有哪些。"""
import glob
import os
import shutil
import sqlite3
import tempfile

BASE = os.path.expanduser("~/Library/Application Support/Google/Chrome")

paths = sorted(glob.glob(os.path.join(BASE, "*", "Network", "Cookies")) +
               glob.glob(os.path.join(BASE, "*", "Cookies")))

for p in paths:
    prof = os.path.basename(os.path.dirname(os.path.dirname(p)))
    if os.path.basename(os.path.dirname(p)) != "Network":
        prof = os.path.basename(os.path.dirname(p))
    tmp = tempfile.mktemp(suffix=".db")
    try:
        shutil.copy2(p, tmp)
        con = sqlite3.connect(tmp)
        rows = con.execute(
            "SELECT host_key,name,length(value),length(encrypted_value),path "
            "FROM cookies WHERE host_key LIKE '%weixin%' ORDER BY host_key,name").fetchall()
        con.close()
        print(f"\n== {prof}  {p.replace(os.path.expanduser('~'), '~')}  命中 {len(rows)}")
        for h, n, lv, le, pa in rows:
            print(f"   {h:24} {n:26} 明文={lv:>4} 密文={le:>5}  {pa}")
    except Exception as e:
        print(f"[{prof}] {e}")
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)
