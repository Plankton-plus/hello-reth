"""数据交易地址及等待交易入块的公共工具。"""

import time

from client.rpc import rpc

DATA_ADDRESS = "0x000000000000000000000000000000000000a900"


def wait_receipts(hashes):
    """等待 hashes 中每笔交易成功入块，最多三十秒；失败立即报告。"""
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        receipts = [rpc("eth_getTransactionReceipt", [tx]) for tx in hashes]
        if all(receipts):
            assert all(receipt["status"] == "0x1" for receipt in receipts)
            return receipts
        time.sleep(0.2)
    raise AssertionError("交易未在三十秒内成功入块")
