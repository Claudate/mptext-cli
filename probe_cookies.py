"""从本机浏览器 cookie 库里取 mp.weixin.qq.com 的登录态。

Safari 的 Cookies.binarycookies 是明文；Chrome 的 Cookies 是 SQLite + Keychain 加密。
只做读取，不改动任何文件。
"""
import glob
import os
import sqlite3
import struct
import sys

DOMAIN = "weixin.qq.com"


def parse_binarycookies(path):
    """Safari binarycookies：返回 [(domain, name, value, path, expires)]。"""
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"cook":
        return []
    num_pages = struct.unpack(">I", data[4:8])[0]
    page_sizes = [struct.unpack(">I", data[8 + 4 * i: 12 + 4 * i])[0]
                  for i in range(num_pages)]
    out, off = [], 8 + 4 * num_pages
    for size in page_sizes:
        page = data[off:off + size]
        off += size
        n = struct.unpack("<I", page[4:8])[0]
        offs = [struct.unpack("<I", page[8 + 4 * i: 12 + 4 * i])[0] for i in range(n)]
        for co in offs:
            c = page[co:]
            size_c = struct.unpack("<I", c[0:4])[0]
            c = c[:size_c]
            # 0..4 size, 4..8 ?, 8..12 flags, 12..16 ?, 16..20 expiry, 20..24 create
            url_off = struct.unpack("<I", c[16:20])[0]
            name_off = struct.unpack("<I", c[20:24])[0]
            path_off = struct.unpack("<I", c[24:28])[0]
            value_off = struct.unpack("<I", c[28:32])[0]

            # 修正：实际布局 offset 从 16 开始是 comment/url/name/path/value
            url_off, name_off, path_off, value_off = (
                struct.unpack("<I", c[16:20])[0],
                struct.unpack("<I", c[20:24])[0],
                struct.unpack("<I", c[24:28])[0],
                struct.unpack("<I", c[28:32])[0],
            )

            def zs(o):
                if o >= len(c):
                    return ""
                e = c.find(b"\x00", o)
                return c[o:e if e >= 0 else len(c)].decode("utf-8", "replace")

            out.append((zs(url_off), zs(name_off), zs(value_off), zs(path_off), 0))
    return out


def chrome_cookies(path):
    """Chrome/Edge：读 SQLite，encrypted_value 尝试用 Keychain 主密钥解。"""
    tmp = "/tmp/_ck_copy.db"
    import shutil
    shutil.copy2(path, tmp)
    con = sqlite3.connect(tmp)
    rows = con.execute(
        "SELECT host_key,name,value,encrypted_value,path,expires_utc "
        "FROM cookies WHERE host_key LIKE ?", (f"%{DOMAIN}%",)).fetchall()
    con.close()
    os.remove(tmp)

    key = None
    try:
        from Crypto.Cipher import AES
        import hashlib
        import subprocess
        pw = subprocess.run(
            ["security", "find-generic-password", "-w", "-s", "Chrome Safe Storage"],
            capture_output=True, timeout=8).stdout.strip()
        if pw:
            key = hashlib.pbkdf2_hmac("sha1", pw, b"saltysalt", 1003, 16)
    except Exception as e:
        print("  (主密钥未取到:", e, ")")

    out = []
    for host, name, val, enc, p, exp in rows:
        if val:
            out.append((host, name, val, p, exp))
        elif enc and key:
            try:
                from Crypto.Cipher import AES
                e = enc[3:] if enc[:3] in (b"v10", b"v11") else enc
                d = AES.new(key, AES.MODE_CBC, b" " * 16).decrypt(e)
                d = d[:-d[-1]]
                out.append((host, name, d.decode("utf-8", "replace"), p, exp))
            except Exception:
                out.append((host, name, "<解密失败>", p, exp))
    return out


cands = [
    ("Safari", "~/Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies", "bc"),
    ("Chrome", "~/Library/Application Support/Google/Chrome/Default/Cookies", "chrome"),
    ("Chrome-Profile1", "~/Library/Application Support/Google/Chrome/Profile 1/Cookies", "chrome"),
]

for label, pat, kind in cands:
    for path in glob.glob(os.path.expanduser(pat)):
        if not os.path.exists(path):
            continue
        try:
            rows = parse_binarycookies(path) if kind == "bc" else chrome_cookies(path)
        except Exception as e:
            print(f"[{label}] 解析失败 {e}")
            continue
        hits = [r for r in rows if DOMAIN in (r[0] or "")]
        print(f"[{label}] {os.path.basename(path)} 全部 {len(rows)} 条，weixin {len(hits)} 条")
        for host, name, val, p, exp in hits:
            show = val if len(val) <= 28 else val[:12] + "..." + val[-6:]
            print(f"   {host:22} {name:18} {show}")
