"""通过 Cast 编码 ABI，通过 eth_call 调用已部署的真实查询合约。"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from client.rpc import rpc

TOOLS = Path("/argus/run/tools")
SIGNATURE = "query((bytes32,uint64,uint64,uint64,uint32[],bytes,bool))"
VERIFIER = "/argus/target/debug/blas-file"


def cast(*arguments):
    """调用固定版本 Cast；arguments 为独立参数，不经过 shell 拼接。"""
    return subprocess.check_output([str(TOOLS / "cast"), *arguments], text=True,
                                   env={**os.environ, "RAYON_NUM_THREADS": "2"}, timeout=30).strip()


def encode_query(request):
    """将 request 的快照范围、日志区间、生成器和布尔表达式编码为 calldata。"""
    fields = [request[name] for name in ("endBlockHash", "startBlock", "timeMin", "timeMax")]
    predicate = "0x" + json.dumps(request["predicate"], separators=(",", ":"), ensure_ascii=False).encode().hex()
    argument = "(" + ",".join(map(str, fields)) + "," + json.dumps(request["generators"]) + "," + predicate + "," + str(request.get("aggregate", True)).lower() + ")"
    return cast("calldata", SIGNATURE, argument)


def call_query(contract, request, anchor, gas=8000000):
    """在固定 anchor 上调用 contract，并返回原始 ABI 结果；gas 限制查询耗气。"""
    return rpc("eth_call", [{"to": contract, "input": encode_query(request), "gas": hex(gas)},
                            {"blockHash": anchor, "requireCanonical": True}])


def decode_response(encoded):
    """解码匹配交易及独立 JSON VO；证明字节不混入交易载荷。"""
    rows, proof = json.loads(cast("abi-decode", "query()(((bytes32,uint32,bytes32,bytes)[],bytes))", encoded, "--json"))[0]
    return {"matches": [{"blockHash": row[0], "transactionIndex": int(row[1]),
                         "transactionHash": row[2], "rawTransaction": row[3]} for row in rows],
            "proof": json.loads(bytes.fromhex(proof[2:])), "proof_bytes": (len(proof) - 2) // 2}


def decode_matches(encoded):
    """返回 encoded 中的交易列表，独立验证由 verify_response 显式执行。"""
    return decode_response(encoded)["matches"]


def business_query(request):
    """把客户端 request 转为 Rust 公共模型，条目谓词仅传给链下文件查询。"""
    return {"generators": request["generators"], "time": [request["timeMin"], request["timeMax"]],
            "predicate": request["predicate"], "entry_predicate": request.get("entryPredicate"),
            "aggregate": request.get("aggregate", True)}


def verify_response(response, request, headers=None):
    """独立进程仅接收可信头、查询、结果和 VO，不访问服务端索引。"""
    from client.headers import authenticated_headers
    value = {"headers": headers if headers is not None else authenticated_headers(request),
             "query": business_query(request), "proof": response["proof"], "matches": response["matches"]}
    with tempfile.TemporaryDirectory(dir="/argus/run") as directory:
        path = Path(directory) / "verify.json"
        path.write_text(json.dumps(value))
        return json.loads(subprocess.check_output([VERIFIER, "verify-chain", str(path)], text=True))


def query_entries(records, request):
    """模拟 SP 生成文件 VO，再用独立命令验证；原始文件不传入验证命令。"""
    found = []
    with tempfile.TemporaryDirectory(dir="/argus/run") as directory:
        directory = Path(directory)
        query, metadata, proof = (directory / name for name in ("query.json", "metadata.json", "proof.json"))
        query.write_text(json.dumps(business_query(request)))
        for record in records:
            value = record["record"]["metadata"]
            metadata.write_text(json.dumps(value))
            source = Path("/argus/shared/blas/files") / (value["omt_root"][2:] + ".json")
            subprocess.check_call([VERIFIER, "query", str(source), str(query), str(proof)])
            entries = json.loads(subprocess.check_output([VERIFIER, "verify", str(metadata), str(query), str(proof)], text=True))
            found.extend({"blockHash": record["block_hash"], "transactionIndex": record["record"]["transaction_index"],
                          "entry": entry} for entry in entries)
    return found


def main():
    """从唯一命令行参数指定的 JSON 文件读取 contract、anchor 和 request 后查询。"""
    if len(sys.argv) != 2:
        raise SystemExit("用法：python3 -m client.query /argus/run/query-example.json")
    example = json.loads(Path(sys.argv[1]).read_text())
    encoded = call_query(example["contract"], example["request"], example["anchor"])
    response = decode_response(encoded)
    records = verify_response(response, example["request"])
    result = {"verified": True, "records": records, "proof_bytes": response["proof_bytes"]}
    if "entryPredicate" in example["request"]:
        result["entries"] = query_entries(records, example["request"])
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
