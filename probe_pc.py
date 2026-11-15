from curl_cffi import requests

BIZ = "MjM5NjAwNzI0MA=="
DESKTOP = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 "
           "(KHTML, like Gecko) Version/18.0 Safari/605.1.15")

r = requests.get(
    f"https://mp.weixin.qq.com/mp/profile_ext?action=home&__biz={BIZ}&scene=124",
    headers={"User-Agent": DESKTOP, "Referer": "https://mp.weixin.qq.com/"},
    impersonate="chrome131", timeout=20)
print("status", r.status_code, "len", len(r.text))
print("=" * 70)
print(r.text)
