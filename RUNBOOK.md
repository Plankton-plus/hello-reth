# Argus 原生区块与方案框架

工程使用 Reth v2.10.0（提交 `f5192ac72d92bc1e656a6939796421af76544af4`）、Rust 1.96.1、Forge/Cast 1.5.0 和 solc 0.8.30。命令在 Ubuntu Bash 输入，编译、安装、生成数据和运行均在 Docker 内完成。

## 结构与调用链

`src/model/` 定义公共元数据和布尔表达式；`src/scheme.rs` 定义方案接口及唯一的 `ActiveScheme` 选择；`src/schemes/bf/` 与 `src/schemes/bf.rs` 是当前具体方案。`src/chain.rs` 提供固定父链读取上下文，`src/node/` 接入 Reth，`src/query/` 提供公共查询入口与预算。

原生头为 `IndexedHeader`，包含公共 Ethereum 执行字段、方案编号、版本和方案自己的 `HeaderFields`。完整头的 RLP 参与区块哈希，并保存到 Reth 头部存储；索引根不再放入 `extra_data`。标准 `eth_getBlockByHash` 也返回方案头字段。

原生体为 `IndexedBody`，包含交易等公共字段和方案自己的 `BodyFields`。当前 BF 的复合体含关键词树及生成器倒排索引；头部含日志时间最小值、最大值、记录数和按固定顺序排列的两个摘要。数据库每个高度保存一个复合值，两个索引一同编码、写入和读回。

```text
构建：交易池 → builder 执行交易 → payload → block 提取统一元数据
      → Scheme::build(父链上下文、元数据) → 原生头与复合体
接收：payload 格式与全头哈希 → 方案承诺检查
      → Scheme::validate(父链上下文、元数据、头体字段)
      → Ethereum 执行校验 → 同事务保存
查询：eth_call → Query.sol → STATICCALL 0xa901
      → 公共参数和历史哈希认证 → Scheme::query(查询上下文、公共查询、预算)
      → 方案决定块间和块内算法 → 返回原始签名交易
```

新增方案实现 `HeaderFields`、`BodyFields`、`build`、`verify_commitment`、`validate` 和 `query`，在 `src/schemes/` 注册模块并修改 `ActiveScheme`。字段类型需满足接口列出的 serde、RLP 和基础类型约束。一条实验链固定一个方案，换方案或协议版本使用新状态卷，创世头中的方案与版本会改变创世哈希。

`BuildContext` 只允许读取父分支祖先。`QueryContext` 只允许读取本次已认证的历史范围，提供头部读取和完整区块读取；框架没有规定逐块扫描。当前逐块检查头部时间、用生成器索引预筛、遍历 BF 树的策略全部位于 BF 方案。BF 对 NOT 保守放行，最终按每份文件的关键词集合精确判断。

## 数据与查询模型

数据交易发往 `0xa900`，input 为 `ARGUS2\0` 后直接拼接 UTF-8 元数据 JSON。信封层只提取文本和真实交易位置。没有额外事件时间前缀，没有按行号生成的时间。JSON 字段固定为 `lid`、`fid`、`ts_min`、`ts_max`、`fw`、`bm_ew`、`rh`；本原型约定 lid 是生成器标识，fid 是文件标识。

时间使用 Unix 秒，表示文件内日志的最早和最晚时间。查询采用闭区间相交语义，关键词区分大小写并去除两侧空白。空生成器数组表示全部生成器。`bm_ew` 与 `rh` 作为公共元数据原样认证和保存，当前 BF 不执行文件内 ADS 或 VO 验证。

元数据 JSON 的结构示例；rh 是 32 字节摘要的十六进制数组，bm_ew 是十六进制位图：

```json
{"lid":"LG_1","fid":"F1","ts_min":1715745600,"ts_max":1715745900,"fw":["FTP"],"bm_ew":"0x01","rh":[]}
```

统一业务查询为生成器集合、日志时间区间、JSON 布尔表达式。`keyword` 的 args 是字符串，`and/or` 的 args 是非空子表达式数组，`not` 的 args 是一个子表达式。例如：

```json
{"op":"and","args":[
  {"op":"keyword","args":"FTP"},
  {"op":"not","args":{"op":"keyword","args":"Admin"}},
  {"op":"or","args":[
    {"op":"keyword","args":"incorrect_pwd"},
    {"op":"keyword","args":"incorrect_usr"}
  ]}
]}
```

合约 ABI 为 `query((bytes32,uint64,uint64,uint64,string[],bytes))`，依次为结束块哈希、起始高度、日志起止时间、生成器数组和表达式 JSON 字节。前两个字段是执行范围；业务查询不需要知道具体索引。`anchor` 单独指定 eth_call 快照，必须晚于结束块。

本轮限制为 dev 链、无叔块、单次最多 32 块且位于调用快照之前最近 256 个祖先块内。输入最多 8192 字节，表达式最多 64 节点、16 层，生成器最多 16 个，各标识和关键词最多 64 字节。单块索引与输出各最多 64 KiB，结果最多 128 条。预编译基础费 5000 Gas，头部读取 500，完整体读取额外收取 5000 加索引每字节 8，输出每字节 8；属于小样本原型计费。

## 构建与工具

小内存机器编译前先停止正在运行的实验节点，一次只运行一条链。首次构建镜像和创建编译容器：

```bash
cd /home/fangua/argus/reth-node
docker build -t argus/reth-dev:0 .
docker run -d --name argus-framework-build --cpus 2 --memory 3g --memory-swap 3g \
  -v "$PWD:/argus/project" -v argus-reth-cargo:/usr/local/cargo \
  -v argus-reth-target:/argus/target -v argus-framework-run:/argus/run \
  argus/reth-dev:0 sleep infinity
docker exec argus-framework-build cargo build --locked --bin indexed-node
docker exec argus-framework-build cargo test --locked --offline --lib
```

首次新状态卷安装工具时执行下一条安装命令；当前卷已经安装，无需重复下载。然后编译合约。

```bash
docker exec argus-framework-build sh /argus/project/install_query_tools.sh
docker exec -e RAYON_NUM_THREADS=2 argus-framework-build \
  /argus/run/tools/forge build --root /argus/project/contracts
```

## 生成统一数据与启动

显式生成一份六条文件元数据，保存到共享卷。运行节点只读挂载该卷；所有导入和验收读取同一个文件。

```bash
docker run --rm -v "$PWD:/argus/project:ro" -v argus-shared:/argus/shared \
  argus/reth-dev:0 python3 -B -m scripts.data
```

首次创建运行容器时执行：

```bash
docker run -d --name argus-body-node --cpus 2 --memory 3g --memory-swap 3g --stop-timeout 60 \
  -p 127.0.0.1:8549:8545 -v "$PWD:/argus/project:ro" \
  -v argus-reth-target:/argus/target:ro -v argus-framework-run:/argus/run \
  -v argus-shared:/argus/shared:ro -e XDG_CACHE_HOME=/argus/run/cache \
  argus/reth-dev:0 sh /argus/project/run_body_node.sh
```

已有运行容器直接启动，日志显示 RPC HTTP server started 后进行导入。

```bash
docker start argus-body-node
docker exec argus-body-node tail -n 15 /argus/run/node.log
docker exec argus-body-node python3 -B -m scripts.demo
docker exec argus-body-node cat /argus/run/query-example.json
docker exec argus-body-node python3 -B -m client.query /argus/run/query-example.json
```

demo 分三批提交六份元数据，另外加入两条不参与索引的交易，然后部署查询合约。默认查询 LG_1，使用上面的 FTP、NOT Admin、密码或用户名错误谓词，返回 F2、F4 对应的两条交易。再次执行 demo 会重新导入和部署；只查询时仅执行 client.query。

查询参数文件在状态卷内，可以在容器内修改其 request；保留 contract、anchor 和快照范围即可复查原快照。返回项包含块哈希、从零开始的交易位置、交易哈希和原始签名交易。正常查询使用 eth_call，不提交查询交易。

## 验收、重启与结果

```bash
docker exec argus-body-node python3 -B -m tests.block_body create
docker exec argus-body-node python3 -B -m tests.query create
docker restart argus-body-node
docker exec argus-body-node tail -n 15 /argus/run/node.log
```

待 RPC 恢复后执行 verify。它使用保存的区块和快照，不重新导入或补建索引。

```bash
docker exec argus-body-node python3 -B -m tests.block_body verify
docker exec argus-body-node python3 -B -m tests.query verify
docker exec argus-body-node cat /argus/run/body-result.json
docker exec argus-body-node cat /argus/run/query-result.json
docker exec argus-body-node python3 -B -m client.query /argus/run/query-example.json
```

`tests.block_body` 独立编码完整头并核对区块哈希、两个索引摘要和日志时间边界。分别篡改索引、缺失索引、同步重算摘要与新块哈希的伪造、伪造头部时间均须被拒绝。`tests.query` 将 10 个正常查询与独立交易扫描比较，检查 12 个拒绝用例，并提交一笔真实查询交易验收执行路径。

`argus-framework-run` 保存本轮数据库、完整索引、工具、开发密钥和结果；`argus-shared` 保存共享元数据；Cargo 和编译缓存分别使用 `argus-reth-cargo`、`argus-reth-target`。旧格式数据仍保留在 `argus-body-run`，本轮不迁移或覆盖它。宿主机 RPC 为 `http://127.0.0.1:8549`，容器内为 `http://127.0.0.1:8545`；8551 的 JWT 实验 Engine RPC 由验收脚本在容器内调用。

完成编译可删除临时容器，所有命名卷保留。停止、重启和后续启动使用：

```bash
docker rm -f argus-framework-build
docker stop argus-body-node
docker start argus-body-node
```

输入非法返回 QueryError；已认证历史缺失或存储损坏中止执行。没有扫描降级、自动补建索引或旧格式兼容层。本轮是 BF 方案接入框架的功能验证，四篇论文算法、VO、完整日志条目 ADS、多节点传播和标准共识客户端互操作尚未验收。

## 本轮实测

2026-11-11：容器内节点及合约编译成功，六项 Rust 单元测试、10 个正常查询、12 个拒绝用例及真实查询交易通过。三块内六份数据记录和两条排除交易正确处理；两份索引分别篡改、同步重算摘要和完整头哈希的伪造、缺失索引和伪造时间均返回 INVALID。

重启后完整头体与保存值一致，10 个固定快照查询 ABI 返回值逐字节一致，并与独立交易扫描再次一致。Rust/Solidity 格式和 14 个 Python 文件语法检查通过。共享样本 SHA256 为 `ac695fcd04f047421347195738f6363c66586e1a8abfee545fbe7b54ae1056db`，当前节点保持运行。
