"""容器内标准 JSON-RPC 客户端，不依赖测试代码。"""

import json
import urllib.request


def rpc(method, params):
    """调用本地节点的 method；params 为位置参数，RPC 错误直接抛出。"""
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    request = urllib.request.Request("http://127.0.0.1:8545", body.encode(),
                                     {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=10) as response:
        result = json.load(response)
    if "error" in result:
        raise RuntimeError(result["error"])
    return result["result"]
