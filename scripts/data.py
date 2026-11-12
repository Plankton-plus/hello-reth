"""在共享卷生成一份确定的小型日志元数据，所有方案和验收共用。"""

import hashlib
import json
from datetime import datetime
from pathlib import Path

SAMPLE = Path("/argus/shared/framework/metadata.jsonl")


def timestamp(value):
    """将带时区的 ISO 时间 value 转为秒级时间戳。"""
    return int(datetime.fromisoformat(value).timestamp())


def generate():
    """显式生成六份文件元数据；时间表示文件内日志区间，内容用于微型验收。"""
    rows = [
        ("LG_1", "F1", "2024-05-15T12:00:00+08:00", "2024-05-15T12:05:00+08:00", ["FTP"]),
        ("LG_1", "F2", "2024-05-15T12:10:00+08:00", "2024-05-15T12:15:00+08:00", ["FTP", "incorrect_pwd"]),
        ("LG_1", "F3", "2024-05-15T12:20:00+08:00", "2024-05-15T12:25:00+08:00", ["FTP", "Admin", "incorrect_pwd"]),
        ("LG_1", "F4", "2024-05-16T10:00:00+08:00", "2024-05-16T10:05:00+08:00", ["FTP", "incorrect_usr"]),
        ("LG_2", "F5", "2024-05-16T10:10:00+08:00", "2024-05-16T10:15:00+08:00", ["FTP", "incorrect_pwd"]),
        ("LG_1", "F6", "2024-05-16T10:20:00+08:00", "2024-05-16T10:25:00+08:00", ["SSH", "incorrect_pwd"]),
    ]
    records = []
    for lid, fid, lower, upper, words in rows:
        content = (fid + ":" + ",".join(words)).encode()
        records.append({"lid": lid, "fid": fid, "ts_min": timestamp(lower), "ts_max": timestamp(upper),
                        "fw": sorted(words), "bm_ew": "0x01", "rh": ["0x" + hashlib.sha256(content).hexdigest()]})
    SAMPLE.parent.mkdir(parents=True, exist_ok=True)
    SAMPLE.write_text("".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records))
    print(json.dumps({"path": str(SAMPLE), "records": len(records), "sha256": hashlib.sha256(SAMPLE.read_bytes()).hexdigest()}))


if __name__ == "__main__":
    generate()
