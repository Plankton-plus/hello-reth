"""在容器内调用认证的实验 Engine RPC；JWT 密钥始终留在状态卷内。"""

import base64
import hashlib
import hmac
import json
import time
import urllib.request
from pathlib import Path


def b64url(data):
    """将字节 data 编码为不带填充的 URL-safe Base64。"""
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def auth_rpc(method, params):
    """用状态卷内 JWT 调用 method，params 是位置参数；不输出密钥或令牌。"""
    key = bytes.fromhex(Path("/argus/run/data/jwt.hex").read_text().strip().removeprefix("0x"))
    header = b64url(b'{"alg":"HS256","typ":"JWT"}')
    claims = b64url(json.dumps({"iat": int(time.time())}).encode())
    message = f"{header}.{claims}"
    signature = b64url(hmac.new(key, message.encode(), hashlib.sha256).digest())
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request("http://127.0.0.1:8551", body,
              {"Content-Type": "application/json", "Authorization": f"Bearer {message}.{signature}"})
    with urllib.request.urlopen(request, timeout=15) as response:
        result = json.load(response)
    if "error" in result:
        raise RuntimeError(result["error"])
    return result["result"]
