# Timeline

给 LLM 对话提供持久化分支、可逆摘要和 stdio MCP 接口的 Rust 项目。`tl` 是 JSON CLI，`tl-mcp` 提供 14 个工具；两者共用带文件锁的 JSON 数据库。Core 无文件 I/O、时钟或网络访问。

构建依据是 [TIMELINE-SPEC v1.1.2](TIMELINE-SPEC.md)。归档按 v1.1.1 修订搬运被摘要覆盖的独有原文，预算按 v1.1.2 契约返回结构化不可行错误；历史修正依据与其余边界解释见 [SPEC-NOTES](docs/SPEC-NOTES.md)。

`archive-branch` 将符合条件的分支独有原文从热区 `nodes` 搬到同一 JSON 数据库的 `archive` 区，保留共享节点、摘要及全部原文内容；它减少热区节点数，不承诺缩小数据库文件。只读查询可直接读取归档内容，显式恢复命令预留给 v1.2。

## 构建与验收

从源码构建需要 Rust/Cargo 及对应的本机链接工具链；也可下载下述 CI 二进制，无需安装 Rust。Cargo 声明 Rust 1.85、edition 2024；本地验收环境为 Windows 上的 Rust/Cargo 1.97.1，未单独验证最低版本；跨平台结果以对应 CI 运行为准。

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

也可用 `python scripts/acceptance.py` 一键运行检查，日志保存到 `target/acceptance/commands.log` 和 `target/acceptance/results.json`。Python 仅用于验收/治理脚本，运行程序不需要它。测试使用临时数据库。

最新检查结果、测试分组及平台范围见 [§13 逐项验收记录](docs/ACCEPTANCE.md)，其中也声明尚未启用的可选优化。

[GitHub Actions CI](.github/workflows/ci.yml) 在 push 和 pull_request 时，使用 stable Rust 在 Ubuntu、macOS、Windows 上分别执行测试、Clippy 严格检查和格式检查，随后构建并上传 Release 二进制。各次远端检查的实际结果见 [Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml)。

### 下载二进制，无需编译

在 [Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml) 打开所需提交的成功运行，在 **Artifacts** 下载对应平台的 ZIP，解压即可取得两个二进制：

| Artifact | ZIP 内的文件 |
|---|---|
| `timeline-windows-latest` | `tl.exe`、`tl-mcp.exe` |
| `timeline-ubuntu-latest` | `tl`、`tl-mcp` |
| `timeline-macos-latest` | `tl`、`tl-mcp` |

这些包按 [GitHub runner](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) 的原生目标构建：当前 Windows/Linux 为 x64，macOS 为 arm64。目标机器须匹配 CPU 架构并使用兼容的系统版本；`-latest` 会更新，以该次运行的 runner 信息为准。

Windows 解压后可直接运行。Linux/macOS 的 artifact [不保留执行权限](https://github.com/actions/upload-artifact/blob/v4/README.md#permission-loss)，解压后先在所在目录运行：

```sh
chmod +x tl tl-mcp
```

然后按 [MCP stdio 接入](#mcp-stdio-接入) 将 `command` 设置为解压后 `tl-mcp`（Windows 为 `tl-mcp.exe`）的绝对路径，并指定数据库路径。使用这些二进制不需要 Rust/Cargo。

### 本地构建产物

构建产物为 `target/release/tl`、`target/release/tl-mcp`，Windows 带 `.exe`。将产物加入 PATH，或直接通过 Cargo 运行：

```sh
cargo run --bin tl -- --db demo.timeline.json create
cargo run --bin tl-mcp -- --db demo.timeline.json
```

程序运行不访问网络、不依赖外部服务、不监听端口；首次下载依赖可能需要网络。

## 路径与 CLI

数据库路径优先级为 **`--db PATH` > 非空 `TIMELINE_DB` > 用户目录中的 `.timeline.json`**。Windows 用户目录取 `USERPROFILE`，缺失时回退 `HOME`；Unix 取 `HOME`。父目录必须已存在。应用不自动加载 `.env`，详见 [环境说明](docs/environments.md)。

所有 CLI 命令均可附带全局参数 `--db PATH`、`--timeline TL`、`--idempotency-key KEY`。省略 timeline 时选择 ID 数字最小、最早创建的 timeline；create 不需要已有 timeline。成功向 stdout 输出 JSON、退出码为 0；错误向 stderr 输出 `{"error":"原因"}`、退出码非零，预算不可行错误另含 `data`（见下文）。`--help`/`--version` 为文本帮助。

下面是完整命令列表。方括号表示可选参数，不要原样输入；ID 应替换为实际返回值。各命令前都可加 `tl --db PATH --timeline TL`。

| CLI | MCP 工具 | 返回形状 |
|---|---|---|
| `create` | `timeline_create` | `{"timeline_id":"tl0"}` |
| `append --role user --text "内容" [--tokens 20]` | `timeline_append` | `{"node":"n1"}` |
| `fork --node n1 --name 分支` | `timeline_fork` | `{"branch":"分支","head":"n1"}` |
| `switch --name 分支` | `timeline_switch` | `{"current_branch":"分支"}` |
| `drop-branch --name 分支` | `timeline_drop_branch` | `{"dropped":"分支"}` |
| `context --node n2` | `timeline_context` | 从旧到新的 Message 数组 |
| `budget --node n2 --max 150` | `timeline_context_budget` | `messages,total_tokens,summaries_created` |
| `pin --node n1` | `timeline_pin` | `{"pinned":"n1"}` |
| `unpin --node n1` | `timeline_unpin` | `{"unpinned":"n1"}` |
| `summarize --from n1 --to n2 --summary "摘要" [--tokens 10] [--derived-from n9,n10] [--supersede n7]` | `timeline_summarize` | `{"node":"n4"}` |
| `branches` | `timeline_branches` | `{name,head,current}` 数组 |
| `node --node n1` | `timeline_node` | 节点详情，role/text/tokens 为顶层字段 |
| `archive-check --name 分支` | `timeline_archive_check` | `eligible,reasons` |
| `archive-branch --name 分支` | `timeline_archive_branch` | `{"archived":["n2","n3"]}`，仅列本次实际搬运的节点 |

MCP 除 create 外均须显式传 timeline_id；CLI `--max` 对应 `max_tokens`，`--derived-from` 对应字符串数组 `derived_from`，归档 `--name` 对应 `branch`。完整参数 schema 由 tools/list 返回。

role 允许 user/assistant/system/summary；节点 ID 为 n 加数字；分支名非空且禁止 `/`，支持 Unicode。单条 text 上限 1 MiB UTF-8，context/budget 输出上限 100000 条消息。tokens 和预算须为非负整数；溢出显式报错。

以下示例假设 demo.timeline.json 尚不存在：

```sh
tl --db demo.timeline.json create
tl --db demo.timeline.json append --role user --text "讨论目标" --tokens 8
tl --db demo.timeline.json append --role assistant --text "明确范围" --tokens 12
tl --db demo.timeline.json fork --node n1 --name 细节
tl --db demo.timeline.json switch --name 细节
tl --db demo.timeline.json append --role user --text "细化验收" --tokens 6
tl --db demo.timeline.json context --node n3
tl --db demo.timeline.json branches
```

## Token、预算、水位与版本

省略 tokens 时，估计器为 **`E(text) = max(1, ceil(Unicode 标量数 / 4))`**，不是模型分词器，不按 UTF-8 字节或字素簇计数。`tokens(node)` 直接读取路径前缀和，O(1)；预算使用每条消息自身的 tokens。

**RESERVE = 32**。自动占位节点的 tokens 恒为 32，不依模板重估。相同父节点和区间的占位复用；summaries_created 也包含复用 ID。原文不会因摘要或预算被删除。

选择顺序为目标节点、pin、真实摘要覆盖节点、时间从新到旧。机械占位不进入 Tier 2，以避免重复预算调用改变优先级。参考贪心算法若因多个摘要缺口超限，采用计算实际缺口成本的修复循环；输入可行时保证输出不超过 `max_tokens + RESERVE`。只有 pin 本身超过预算才适用规格的例外。

输入不可行时返回精确错误 `budget infeasible`，`data.required_tokens` 是保留目标、pin 并覆盖全部省略区间的最低布局总 token 数，`data.max_tokens` 是请求预算。失败不提交节点、计数器、审计或幂等结果。规格反例 `[1000, 1(pin), 1000, 0(target)]`、预算 1 的 CLI 错误为 `{"error":"budget infeasible","data":{"required_tokens":65,"max_tokens":1}}`。该契约由 §5.5 明确规定；最低成本计算与回归设计见 [SPEC-NOTES](docs/SPEC-NOTES.md)。

水位表示 root→watermark 路径上的非根节点均被 pin 或摘要覆盖；成功创建摘要/占位后，满足完整覆盖且不回退才推进。若 unpin 会导致水位内出现裸消息，操作报错。

**水位扫描优化尚未启用。** `context_budget_with_watermark(..., true/false)` 均执行相同完整扫描；一致性测试不代表加速路径已实现。

summarize 新增真实摘要，不改写占位或旧摘要；同区间真实摘要解除旧占位的未完成归档依赖。相同区间、文本、tokens、依赖和被替代版本的重复调用复用首次结果。`--supersede` 创建递增版本并指向旧版，旧节点保留；引用旧版的 derived_from 下游级联标记 stale。用 `node --node 旧摘要ID` 可取回旧版，用 `context --node 原区间终点` 可取回原文。

## 归档与冷热读取

归档仍须同时满足水位、整条路径无 pin、无未回填占位依赖三条件，并且不能归档当前分支。成功后删除分支指针，仅搬运路径上仍在热区、分支引用计数为 1、无 pin 的原文节点；摘要覆盖不再阻止搬运，摘要节点始终留在热区。共享前缀、其他分支引用的节点和既有归档节点不重复搬运。`archived` 可以为空，取决于该分支是否有可搬运独有原文。

归档不改写 `parent`、`children` 或消息，因此热摘要的父节点可以位于冷区。`node`、`context` 以及核心的 `tokens`、路径查询统一读取两区，审计中的原文 ID 在重启后仍可追溯。直接从冷节点 `fork` 会报错；可以从保留在热区的摘要 `fork` 并继续追加，其路径中的归档祖先保持可读。显式将冷节点恢复到热区的操作尚未实现。

## 幂等、存储与派生数据

可重试变更应附带稳定幂等键，例如：

```sh
tl --db demo.timeline.json --timeline tl0 --idempotency-key request-001 append --role user --text "一次写入"
```

同 timeline + key 在首次成功结果创建后 **7 天（604800 秒）** 内直接返回首次结果，不重执行、不写快照；进程重启仍有效。create 使用数据库级创建作用域。不要给不同逻辑请求复用 key。失败不占用 key；过期项在成功提交时清理；结果映射写入审计。

Storage 对稳定的 `<db>.lock` 持共享读锁或独占写锁；独占锁覆盖读取最新快照、执行、验证及写入，支持独立进程。成功变更写同目录私有临时文件、fsync、原子替换，并把上一快照保存为 `<db>.bak`。损坏 JSON、无效源结构或不支持的 format_version 拒绝写入并提示恢复。

**未实现可选增量 checkpoint 日志，也没有后台 checkpoint 定时器。** 当前每次成功变更均保存全量 JSON 快照。

Unix 新文件为 0600；Windows 创建数据库、备份、锁时使用受保护 owner-only DACL `D:P(A;;FA;;;OW)`，不继承父目录访问项。Windows 权限、原子快照和跨进程并发已实测；Unix 有对应测试但本次未执行。管理员仍拥有操作系统管理权限。内容未加密，应放在受信任的本地目录并避免提交真实数据。

| 派生字段/索引 | 重建方式 |
|---|---|
| prefix_tokens | 根据 parent 链重算消息前缀和 |
| covers、真实摘要语义覆盖 | 根据摘要区间重建；机械占位不进入语义索引 |
| refcount | 按分支路径数、摘要覆盖数与 pin 查询计算 |
| 占位复用、真实摘要区间、摘要参数索引 | 根据现有摘要节点重建 |
| 版本后继、反向依赖、stale | 根据 supersedes/derived_from 重建并传播 |
| 水位路径集合 | 根据持久化 watermark 与 parent 重建 |

读取时可丢弃任意损坏类型的 prefix_tokens/stale 并重建；源数据损坏仍报错。派生重建和结构验证同时覆盖热区与归档区。format_version 保持 1，缺少 v1.1 新字段的旧文件按规格缺省解释；本次规格修订不改变数据库格式或 MCP 工具集版本。

## 审计与恢复

审计位于 **DB 顶层 audit 数组**。每次真实摘要、修正、新占位创建均记录 Unix 秒 ts、操作、timeline、区间和节点；归档、幂等映射也记日志。Storage 注入不递减时间并拒绝改写/删除旧审计。占位复用不是创建，不重复写创建审计。

按审计 from/to 执行 `node --node FROM` 和 `context --node TO` 可取回原文，旧摘要可按记录 ID 查询。若整个文件损坏，先停用该数据库的 CLI/MCP 进程、保留损坏文件，再显式恢复 .bak。例如手动运行以下 PowerShell（请替换路径）：

```powershell
$timelineDb = 'C:\timeline-data\conversation.timeline.json'
$savedBroken = "$timelineDb.corrupt-$(Get-Date -Format yyyyMMdd-HHmmss)"
Copy-Item -LiteralPath $timelineDb -Destination $savedBroken -ErrorAction Stop
Copy-Item -LiteralPath "$timelineDb.bak" -Destination $timelineDb -ErrorAction Stop
tl --db $timelineDb branches
```

恢复回到上一次成功变更前的快照；首次写入没有旧快照，因此没有 .bak。程序不会自动覆盖损坏文件。

## MCP stdio 接入

客户端启动 tl-mcp，以 stdin/stdout 交换每行一个 UTF-8 JSON-RPC 2.0 消息。stdout 仅为协议，单帧上限 8 MiB；超限或非法 UTF-8 返回协议错误并继续处理后续帧。以下为通用启动信息，请按所用客户端映射外层配置，不是某个特定客户端的完整配置：

```json
{
  "command": "C:\\timeline\\tl-mcp.exe",
  "args": ["--db", "C:\\timeline-data\\conversation.timeline.json"],
  "env": {}
}
```

Unix 替换为对应绝对路径。也可省略 --db，通过进程环境 TIMELINE_DB 指定位置。握手后按以下顺序调用；将 tl0 替换为 create 的实际结果：

```jsonl
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"example","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"timeline_create","arguments":{},"idempotency_key":"new-conversation-001"}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"timeline_append","arguments":{"timeline_id":"tl0","role":"user","text":"你好","idempotency_key":"message-001"}}}
```

支持 ping。初始化报告服务名 timeline、版本 1.1.0；支持协商 2024-11-05、2025-03-26、2025-06-18、2025-11-25，未知版本返回 2025-11-25 供客户端决定是否继续，不承诺未来协议功能。旧两个版本支持 batch 串行处理；2025-06-18 及以后只接受单请求。遵循 [MCP 生命周期](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)、[stdio 传输](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)与[旧版基础协议](https://modelcontextprotocol.io/specification/2025-03-26/basic)。

所有版本的成功结果均在 `result.content[0].text` 中保存工具 JSON 字符串；2025-06-18 及以后的对象结果同时返回 structuredContent。幂等键可在 arguments 或 params 中，两处同时提供须一致。领域错误 code=-32000，协议错误使用对应标准错误码；通知不回应也不执行变更工具。预算不可行返回 `error` 对象 `{"code":-32000,"message":"budget infeasible","data":{"required_tokens":65,"max_tokens":1}}`（此数值对应上文反例）。

更多信息：[技术事实](TECH.md)、[文档索引](docs/README.md)、[环境说明](docs/environments.md)、[机器可读检查命令](.project-governance.json)。
