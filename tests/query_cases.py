"""小样本合约查询验收：与独立交易扫描逐项对照，并验证明确失败。"""

import json
from client.query import call_query, cast, decode_response, verify_response
from client.rpc import rpc
from scripts.data import SAMPLE
from tests.scan import scan_transactions


def keyword(word):
    """将 word 构造成公共布尔语法中的关键词叶节点。"""
    return {"op": "keyword", "args": word}


def check_case(name, contract, request, anchor, count):
    """在 anchor 查询 request，要求结果与扫描一致且条数为 count，并保存原始 ABI。"""
    encoded = call_query(contract, request, anchor)
    response = decode_response(encoded)
    matches = response["matches"]
    verify_response(response, request)
    assert matches == scan_transactions(request), f"{name} 与独立扫描不一致"
    assert len(matches) == count, f"{name} 命中数量错误"
    return {"name": name, "request": request, "anchor": anchor,
            "count": count, "result": encoded, "matches": matches,
            "proof_bytes": response["proof_bytes"]}


def valid_cases(contract, request, anchor, data_hashes):
    """检查混合布尔、区间边界、生成器隔离、假阳性和跨块结果。"""
    boundary = json.loads(SAMPLE.read_text().splitlines()[1])["t_max"]
    variants = [
        ("mixed_boolean", {}, 2),
        ("closed_time_boundary", {"timeMin": boundary, "timeMax": boundary}, 1),
        ("other_generator", {"generators": [2]}, 1),
        ("all_ftp", {"generators": [], "predicate": keyword("FTP")}, 5),
        ("not_admin", {"predicate": {"op": "not", "args": keyword("Admin")}}, 4),
        ("no_match", {"predicate": {"op": "and", "args": [keyword("Admin"), keyword("incorrect_usr")]}}, 0),
        ("normalized", {"generators": [1, 1], "predicate": keyword(" FTP ")}, 4),
        ("missing_keyword", {"predicate": keyword("unknown")}, 0),
        ("not_missing", {"predicate": {"op": "not", "args": keyword("unknown")}}, 5),
        ("all_records", {"generators": [], "predicate": {"op": "or", "args": [keyword("FTP"), keyword("SSH")]}}, 6),
    ]
    cases = [check_case(name + ("_alis" if aggregate else "_native"), contract,
                       {**request, **changes, "aggregate": aggregate}, anchor, count)
             for name, changes, count in variants for aggregate in (False, True)]
    receipt = rpc("eth_getTransactionReceipt", [data_hashes[1]])
    single = {**request, "startBlock": int(receipt["blockNumber"], 16), "endBlockHash": receipt["blockHash"]}
    cases.append(check_case("single_block", contract, single, anchor, 1))
    return cases


def expect_failure(name, contract, request, anchor, gas=2000000, query_error=True):
    """请求应因 QueryError 或明确耗气失败被拒绝，不能将通信故障算作通过。"""
    try:
        call_query(contract, request, anchor, gas)
    except RuntimeError as error:
        detail = error.args[0]
        assert isinstance(detail, dict), "非 RPC 执行错误"
        if query_error:
            selector = cast("sig", "QueryError(string)")
            assert str(detail.get("data", "")).startswith(selector), f"{name} 未返回 QueryError：{detail}"
        else:
            assert any(word in detail["message"].lower() for word in ("gas", "revert")), detail
        return {"name": name, "status": "REJECTED", "error": detail}
    raise AssertionError(f"{name} 未被拒绝")


def invalid_cases(contract, request, anchor):
    """检查参数、32 块跨度、祖先边界、未知块及耗气限制。"""
    end = int(rpc("eth_getBlockByHash", [request["endBlockHash"], False])["number"], 16)
    variants = [
        ("reversed_time", {"timeMin": request["timeMax"] + 1}),
        ("zero_generator", {"generators": [0]}),
        ("unknown_generator", {"generators": [4097]}),
        ("too_many_generators", {"generators": [1] * 17}),
        ("empty_keyword", {"predicate": {"op": "keyword", "args": " "}}),
        ("long_keyword", {"predicate": {"op": "keyword", "args": "a" * 65}}),
        ("unknown_operator", {"predicate": {"op": "xor", "args": []}}),
        ("empty_and", {"predicate": {"op": "and", "args": []}}),
        ("too_many_nodes", {"predicate": {"op": "and", "args": [{"op": "keyword", "args": "FTP"}] * 65}}),
        # 已有足够高度时测试 33 块跨度，新链测试禁止查询创世块。
        ("invalid_block_span", {"startBlock": end - 32 if end > 32 else 0}),
        ("unknown_block", {"endBlockHash": "0x" + "00" * 32}),
        ("current_block", {"endBlockHash": anchor}),
    ]
    results = [expect_failure(name, contract, {**request, **changes}, anchor)
               for name, changes in variants]
    results.append(expect_failure("insufficient_gas", contract, request, anchor, gas=10000, query_error=False))
    return results
