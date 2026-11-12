"""部署真实查询合约，验证跨块查询、明确拒绝及重启后相同快照的结果。"""

import json
import copy
import subprocess
import sys
from pathlib import Path

from client.query import call_query, decode_matches, decode_response, encode_query, query_entries, verify_response
from client.rpc import rpc
from client.transactions import wait_receipts
from scripts.demo import setup_demo
from tests.query_cases import check_case, invalid_cases, valid_cases
from tests.scan import scan_transactions

SAVED = Path("/argus/run/query-result.json")


def reject_forged_proofs(response, request):
    """独立验证器须拒绝遗漏区块、遗漏结果、替换原始交易和伪空结果。"""
    from client.headers import authenticated_headers
    headers = authenticated_headers(request)
    forged = []
    missing_block = copy.deepcopy(response)
    missing_block["proof"]["groups"].pop()
    forged.append(missing_block)
    missing_result = copy.deepcopy(response)
    missing_result["matches"].pop()
    forged.append(missing_result)
    replaced = copy.deepcopy(response)
    replaced["matches"][0]["rawTransaction"] += "00"
    forged.append(replaced)
    empty = {"matches": [], "proof": {"groups": []}}
    forged.append(empty)
    for value in forged:
        try:
            verify_response(value, request, headers)
        except subprocess.CalledProcessError:
            continue
        raise AssertionError("独立验证器接受伪造 VO")
    return len(forged)


def create():
    """分块导入样本、部署合约、比较扫描结果，再实际执行一笔合约查询交易。"""
    demo = setup_demo()
    request, contract, deployment = demo["request"], demo["contract"], demo["deployment"]
    anchor = deployment["blockHash"]
    cases = valid_cases(contract, request, anchor, demo["data_hashes"])
    rejected = invalid_cases(contract, request, anchor)
    tx = rpc("eth_sendTransaction", [{"from": demo["sender"], "to": contract, "input": encode_query(request),
                                      "gas": "0x7a1200", "gasPrice": "0x3b9aca00"}])
    receipt = wait_receipts([tx])[0]
    empty = {**request, "endBlockHash": anchor, "startBlock": int(deployment["blockNumber"], 16)}
    cases.append(check_case("empty_block", contract, empty, receipt["blockHash"], 0))
    response = decode_response(cases[1]["result"])
    assert any(group["kind"] == "window" for group in response["proof"]["groups"]), "未实际走聚合路径"
    records = verify_response(response, request)
    entries = query_entries(records, {**request, "entryPredicate": {"op": "keyword", "args": "error"}})
    assert len(entries) == 4, "日志条目级查询或文件 VO 错误"
    proof_rejections = reject_forged_proofs(response, request)
    result = {"contract": contract, "anchor": anchor, "deployment": deployment["transactionHash"],
              "query_transaction": tx, "cases": cases, "rejected": rejected, "entries": entries,
              "proof_rejections": proof_rejections}
    SAVED.write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps({"phase": "create", "status": "PASS", "contract": contract,
                      "valid_cases": len(cases), "rejected_cases": len(rejected),
                      "verified_entries": len(entries),
                      "rejected_proofs": proof_rejections,
                      "query_transaction": tx, "query_gas_used": int(receipt["gasUsed"], 16)}))


def verify():
    """重启后按每个已保存 anchor 重查，要求原始 ABI 和独立扫描结果保持一致。"""
    saved = json.loads(SAVED.read_text())
    for case in saved["cases"]:
        encoded = call_query(saved["contract"], case["request"], case["anchor"])
        assert encoded == case["result"], f"{case['name']} 重启后结果改变"
        assert decode_matches(encoded) == scan_transactions(case["request"])
        verify_response(decode_response(encoded), case["request"])
    for tx in (saved["deployment"], saved["query_transaction"]):
        receipt = rpc("eth_getTransactionReceipt", [tx])
        assert receipt and receipt["status"] == "0x1"
    print(json.dumps({"phase": "verify", "status": "PASS", "contract": saved["contract"],
                      "valid_cases": len(saved["cases"])}))


def main():
    """create 部署并验收，verify 验证状态卷中已有合约与查询快照。"""
    if len(sys.argv) != 2 or sys.argv[1] not in ("create", "verify"):
        raise SystemExit("用法：python3 -m tests.query create|verify")
    if sys.argv[1] == "create":
        create()
    else:
        verify()


if __name__ == "__main__":
    main()
