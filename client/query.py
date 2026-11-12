"""通过 Cast 编码 ABI，通过 eth_call 调用已部署的真实查询合约。"""

import json
import os
import subprocess
import sys
from pathlib import Path

from client.rpc import rpc

TOOLS = Path("/argus/run/tools")
SIGNATURE = "query((bytes32,uint64,uint64,uint64,string[],bytes))"


def cast(*arguments):
    """调用固定版本 Cast；arguments 为独立参数，不经过 shell 拼接。"""
    return subprocess.check_output([str(TOOLS / "cast"), *arguments], text=True,
                                   env={**os.environ, "RAYON_NUM_THREADS": "2"}, timeout=30).strip()


def encode_query(request):
    """将 request 的快照范围、日志区间、生成器和布尔表达式编码为 calldata。"""
    fields = [request[name] for name in ("endBlockHash", "startBlock", "timeMin", "timeMax")]
    predicate = "0x" + json.dumps(request["predicate"], separators=(",", ":"), ensure_ascii=False).encode().hex()
    argument = "(" + ",".join(map(str, fields)) + "," + json.dumps(request["generators"]) + "," + predicate + ")"
    return cast("calldata", SIGNATURE, argument)


def call_query(contract, request, anchor, gas=2000000):
    """在固定 anchor 上调用 contract，并返回原始 ABI 结果；gas 限制查询耗气。"""
    return rpc("eth_call", [{"to": contract, "input": encode_query(request), "gas": hex(gas)},
                            {"blockHash": anchor, "requireCanonical": True}])


def decode_matches(encoded):
    """解码 encoded 中的匹配交易，保留原始签名交易和块内位置。"""
    rows = json.loads(cast("abi-decode", "query()((bytes32,uint32,bytes32,bytes)[])", encoded, "--json"))[0]
    return [{"blockHash": row[0], "transactionIndex": int(row[1]),
             "transactionHash": row[2], "rawTransaction": row[3]} for row in rows]


def main():
    """从唯一命令行参数指定的 JSON 文件读取 contract、anchor 和 request 后查询。"""
    if len(sys.argv) != 2:
        raise SystemExit("用法：python3 -m client.query /argus/run/query-example.json")
    example = json.loads(Path(sys.argv[1]).read_text())
    encoded = call_query(example["contract"], example["request"], example["anchor"])
    print(json.dumps(decode_matches(encoded), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
