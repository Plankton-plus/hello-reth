"""在共享卷生成一份确定的小型日志元数据，所有方案和验收共用。"""

import hashlib
import json
import subprocess
from pathlib import Path

SAMPLE = Path("/argus/shared/blas/metadata.jsonl")
FILES = SAMPLE.parent / "files"
INPUTS = SAMPLE.parent / "inputs.jsonl"
TOOL = "/argus/target/debug/blas-file"


def generate():
    """从已有共用样本转换六份 BLAS 文件，全部 ADS 由容器内 Rust 工具生成。"""
    source = Path("/argus/shared/records.jsonl")
    rows = [json.loads(line) for line in source.read_text().splitlines()][:24]
    assert len(rows) == 24, "共用样本至少需要 24 条记录"
    public = [["FTP"], ["FTP", "incorrect_pwd"], ["FTP", "Admin", "incorrect_pwd"],
              ["FTP", "incorrect_usr"], ["FTP", "incorrect_pwd"], ["SSH", "incorrect_pwd"]]
    records = []
    inputs = []
    FILES.mkdir(parents=True, exist_ok=True)
    temporary = Path("/argus/run/generator-input.json")
    for index, words in enumerate(public):
        group = rows[index * 4:index * 4 + 4]
        times = [group[i]["timestamp"] for i in (0, 0, 3, 2)]
        entries = [{"sn": i, "timestamp": times[i], "keywords": sorted([row["key"], "error" if i % 2 == 0 else "debug"]),
                    "log_value": row["value"]} for i, row in enumerate(group)]
        value = {"lid": 2 if index == 4 else 1, "tx_public_keywords": words, "entries": entries}
        temporary.write_text(json.dumps(value))
        path = FILES / "building.json"
        metadata = json.loads(subprocess.check_output([TOOL, "build", str(temporary), str(path)], text=True))
        if index == 0:
            Path("/argus/run/file-input.json").write_text(json.dumps(value))
            Path("/argus/run/file-example.json").write_bytes(path.read_bytes())
            Path("/argus/run/file-metadata.json").write_text(json.dumps(metadata))
            query = {"generators": [], "time": [metadata["t_min"], metadata["t_max"]],
                     "predicate": {"op": "all"}, "entry_predicate": {"op": "keyword", "args": "error"}, "aggregate": False}
            Path("/argus/run/file-query.json").write_text(json.dumps(query))
        path.replace(FILES / (metadata["omt_root"][2:] + ".json"))
        records.append(metadata)
        inputs.append(value)
    SAMPLE.write_text("".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records))
    INPUTS.write_text("".join(json.dumps(value) + "\n" for value in inputs))
    result = {"path": str(SAMPLE), "files": 6, "entries": 24,
              "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "sha256": hashlib.sha256(SAMPLE.read_bytes()).hexdigest()}
    (SAMPLE.parent / "manifest.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result))


if __name__ == "__main__":
    generate()
