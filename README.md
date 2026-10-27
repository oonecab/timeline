# Timeline

Timeline 用来保存 LLM 对话、探索不同分支，并在压缩上下文后保留可回看的原文。它提供两个程序：`tl` 供终端和脚本使用，`tl-mcp` 供 MCP 客户端使用；两者共用本地 JSON 数据库。

Windows、Linux、Apple Silicon Mac 和 Intel Mac 使用各自的二进制包，下载后无需安装 Rust。程序运行时不联网、不监听端口，也不依赖后台服务。

第一次使用可以从下面的下载步骤开始；接入客户端见 [MCP 配置](#mcp-stdio-接入)，路径与权限见 [环境说明](docs/environments.md)。实现遵循 [TIMELINE-SPEC v1.2.0](TIMELINE-SPEC.md)，设计边界与历史修正见 [SPEC-NOTES](docs/SPEC-NOTES.md)。

## 下载与开始使用

### 选择系统和芯片对应的包

优先打开 [Releases](https://github.com/oonecab/timeline/releases)，选择所需版本，在 **Assets** 下载系统和芯片对应的包。每个包解压后只有两个二进制文件：

| 目标机器 | CPU 架构 | Release 资产 | 包内文件 |
|---|---|---|---|
| Windows | x64 | `timeline-windows-latest.zip` | `tl.exe`、`tl-mcp.exe` |
| Linux | x64 | `timeline-ubuntu-latest.tar.gz` | `tl`、`tl-mcp` |
| Apple Silicon Mac | arm64 | `timeline-macos-latest.tar.gz` | `tl`、`tl-mcp` |
| Intel Mac | x64 | `timeline-macos-15-intel.tar.gz` | `tl`、`tl-mcp` |

如果所需提交尚未发布，也可以在 [CI Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml) 的成功运行中下载 **Artifacts**。这些未发布构建使用相同的平台名，例如 `timeline-macos-15-intel`，由 Actions 打包为 ZIP。GitHub 自动附带的 **Source code** 包是源码，不能直接当作二进制使用。

包按对应 runner 的原生目标构建，目标机器需要匹配 CPU 架构和兼容的系统版本。Linux 包不是所有发行版通用的静态包，两个 Mac 包也不是 Universal Binary；runner 上通过测试不等于验证过所有旧系统。`-latest` 会更新，具体构建环境可查看那次运行的日志。

`macos-15-intel` 当前可用，但已有退出计划，不能把它当作长期不变的分发渠道。Intel 包解决的是 x64 机器的运行需求；CI 也会在这个 Intel runner 上完整运行测试、Clippy 和格式检查，再构建上传。生命周期公告与兼容范围见 [环境说明](docs/environments.md#ci-平台与兼容范围)。

### Windows：解压后启动

把两个 `.exe` 放到一个固定目录，在该目录打开 PowerShell。下面会显示帮助，并在用户目录下创建一个数据库；最后一步只需在第一次使用时执行：

```powershell
.\tl.exe --help
New-Item -ItemType Directory -Force "$env:USERPROFILE\timeline-data" | Out-Null
.\tl.exe --db "$env:USERPROFILE\timeline-data\conversation.timeline.json" create
```

### Linux / macOS：解压后启动

Release 的 `.tar.gz` 保留执行权限，正常解压后即可启动。下面以 Linux 包为例；Mac 使用表中对应的文件名。命令适用于 bash 和 zsh：

```sh
tar -xzf timeline-ubuntu-latest.tar.gz
./tl --help
mkdir -p "$HOME/.local/share/timeline"
./tl --db "$HOME/.local/share/timeline/conversation.timeline.json" create
```

如果下载的是 Actions artifact ZIP，解压后先运行 `chmod +x tl tl-mcp`，再执行 `./tl --help`。Actions 的 ZIP [不保留执行权限](https://github.com/actions/upload-artifact/blob/v4/README.md#permission-loss)。

不确定 Mac 的芯片类型时，可以查看“关于本机”，或在原生终端执行 `uname -m`：`arm64` 选 Apple Silicon 包，`x86_64` 选 Intel 包。Rosetta 终端可能报告仿真架构，以实际芯片为准。

随后将 MCP 客户端的 `command` 指向这个固定目录里的 `tl-mcp`。若希望在任意目录输入 `tl`，可以把二进制所在目录加入 PATH；配置方法因 shell 而异，MCP 使用绝对路径则不依赖 PATH。

## 从源码构建与检查

开发或自行编译才需要 Rust/Cargo，以及系统对应的本机链接工具链。项目使用 Rust 2024 edition，Cargo 声明最低 Rust 1.85；CI 使用 stable，最低版本未单独验证。源码构建在四类平台使用相同命令：

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

也可以运行 `python scripts/acceptance.py`；Linux/macOS 上如果 Python 命令名为 `python3`，使用 `python3 scripts/acceptance.py`。脚本会把输出写入 `target/acceptance/`。Python 只用于验收脚本，运行下载的二进制不需要它。

[CI 工作流](.github/workflows/ci.yml) 在 push 和 pull_request 时，对表中四个 runner 执行相同的 test、clippy、fmt、Release 构建及产物上传步骤。测试使用临时数据库，具体覆盖范围和已确认的结果见 [验收记录](docs/ACCEPTANCE.md)。

[Release 工作流](.github/workflows/release.yml) 在推送 tag 时运行相同的四平台检查，全部成功后才发布二进制和自动生成的发布说明。维护者的打 tag 步骤、重跑行为和权限说明见 [发布指南](docs/releases.md)。

本机构建产物在 `target/release/`：Windows 是 `tl.exe`、`tl-mcp.exe`，Linux/macOS 是 `tl`、`tl-mcp`。开发时也可通过 Cargo 启动：

```sh
cargo run --bin tl -- --db demo.timeline.json create
cargo run --bin tl-mcp -- --db demo.timeline.json
```

首次下载 Cargo 依赖需要网络；这不影响二进制在运行时离线使用。

## 路径与 CLI

数据库路径优先级为 **`--db PATH` > 非空 `TIMELINE_DB` > 用户目录中的 `.timeline.json`**。Windows 用户目录取 `USERPROFILE`，缺失时回退 `HOME`；Unix 取 `HOME`。父目录必须已存在。应用不自动加载 `.env`，详见 [环境说明](docs/environments.md)。

CLI 支持全局参数 `--db PATH`、`--timeline TL`、`--idempotency-key KEY`；只读导出 `export` 不接受幂等键。省略 timeline 时选择 ID 数字最小、最早创建的 timeline；create 不需要已有 timeline。成功向 stdout 输出 JSON、退出码为 0；`export --format ascii` 省略 `--output` 时直接输出文本图。错误向 stderr 输出 `{"error":"原因"}`、退出码非零，预算不可行错误另含 `data`（见下文）。`--help`/`--version` 为文本帮助。

下面的示例假设 `tl` 已在 PATH 中。若仍从解压目录运行，Windows 将 `tl` 换为 `.\tl.exe`，Linux/macOS 换为 `./tl`。方括号表示可选参数，节点和时间线 ID 用实际返回值替换。

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
| `export --format drawio --output task.drawio` | 无，仅 CLI | `format,timeline_id,output,nodes,edges` |
| `export --format ascii [--output task.txt]` | 无，仅 CLI | 省略 output 时为文本图；保存文件时为上述 JSON |

MCP 除 create 外均须显式传 timeline_id；CLI `--max` 对应 `max_tokens`，`--derived-from` 对应字符串数组 `derived_from`，归档 `--name` 对应 `branch`。完整参数 schema 由 tools/list 返回。

`append` 和 `summarize` 还可附带 `--intent-tag` 与 `--intent-note`；MCP 中对应可选 `intent` 对象，示例见下节。

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

### 给消息或摘要加意图标注

想区分“提问”“决策”“执行”等内容，可以在写入时加一个标签。词表由你决定，不加标签也能照常使用。下面的命令在 PowerShell、bash 和 zsh 中都适用，仍按前文选择 `tl`、`.\tl.exe` 或 `./tl`：

```sh
tl --db demo.timeline.json append --role user --text "先完成本地版本" --intent-tag "决策" --intent-note "暂时不需要云同步"
tl --db demo.timeline.json summarize --from n1 --to n2 --summary "已明确目标和范围" --intent-tag "阶段总结"
```

`--intent-note` 必须和 `--intent-tag` 一起使用。对应的 MCP `timeline_append` 参数为：

```json
{
  "timeline_id": "tl0",
  "role": "user",
  "text": "先完成本地版本",
  "intent": {"tag": "决策", "note": "暂时不需要云同步"}
}
```

标签必须是非空字符串；备注可以省略。标注写入后不能修改，归档后仍保留。`context` 和 `budget` 中，有标注的消息会多一个 `intent_tag` 字段，无标注的消息维持原来的形状；备注不会进入上下文，需要时用 `node` / `timeline_node` 查看完整 `intent`。

摘要的区间、文本及其他参数相同，但标签或备注不同，会保存为不同节点；完全相同的请求在重启后仍复用原节点。显式 `--supersede` 继续遵循摘要版本链规则；使用同一幂等键则始终返回首次结果。预算仍按消息的 `tokens` 计算，不另计标签开销。详细契约见 [规格 §3–4](TIMELINE-SPEC.md#3-数据模型)。

## 导出可编辑的节点图

```sh
tl --db demo.timeline.json --timeline tl0 export --format drawio --output task.drawio
```

用 draw.io 打开生成的 `.drawio` 文件，即可拖动节点、调整连线、修改图中文字和另存文件。图采用从左到右的圆角节点布局，当前 head 为绿色，普通消息为浅蓝色；摘要、pin 与归档节点有单独标记。完整原文保存在节点数据的 `text_json` 属性中。

导出包含所选时间线的全部冷热节点，是独立的历史快照；不改写数据库，不启动网页服务，不增加 MCP 工具，修改图文件也不会回写 Timeline。已有输出文件会报错，请换一个文件名保留手工调整。摘要覆盖、版本与依赖在默认隐藏的独立图层中，可在 draw.io 的图层面板开启。

完整规则、打开方式和测试范围见 [draw.io 导出说明](docs/drawio.md)；可以先打开 [合成示例](examples/timeline.drawio) 查看样式。

## 在 Bash / PowerShell 中查看节点图

```sh
tl --db demo.timeline.json --timeline tl0 export --format ascii
tl --db demo.timeline.json --timeline tl0 export --format ascii --output task.txt
```

第一条直接在终端打印分支树；第二条保存 UTF-8 文本文件并输出 JSON 结果。连线使用 `|`、`+--`、`` `-- ``，消息保留中文，连续历史不会逐层缩进；当前分支、pin、归档、摘要版本与覆盖关系均有标记。示例：

```text
[n1 user PIN] "只使用本地数据"
|
+-- [n2 assistant] "方案 A"
|   |
|   [n3 assistant] "验证结果" [HEAD* "main"]
`-- [n4 assistant] "方案 B" [HEAD "alternative"]
```

ASCII 输出是面向人的预览，消息和分支名超过 64 个 Unicode 标量时用 `...` 标记省略；完整原文仍可通过 `tl node --node ID` 查询。支持管道和分页器，原有命令的 JSON 输出保持不变。Bash、PowerShell 的命令示例、文本编码和输出约定见 [终端图说明](docs/ascii.md)。

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

Linux/macOS 新建的数据库、备份和锁文件权限为 `0600`；Windows 使用受保护的仅所有者可访问 DACL，不继承父目录访问项。对应权限、原子写入和并发回归由各平台 CI 执行，结果见 [验收记录](docs/ACCEPTANCE.md)。文件内容没有加密，管理员仍可通过操作系统管理权限访问；请放在可信目录，也不要把真实对话提交到仓库。

| 派生字段/索引 | 重建方式 |
|---|---|
| prefix_tokens | 根据 parent 链重算消息前缀和 |
| covers、真实摘要语义覆盖 | 根据摘要区间重建；机械占位不进入语义索引 |
| refcount | 按分支路径数、摘要覆盖数与 pin 查询计算 |
| 占位复用、真实摘要区间、摘要参数索引 | 根据现有摘要节点重建，摘要参数索引包含完整 intent |
| 版本后继、反向依赖、stale | 根据 supersedes/derived_from 重建并传播 |
| 水位路径集合 | 根据持久化 watermark 与 parent 重建 |

读取时会丢弃损坏的 prefix_tokens/stale 并重建；源数据损坏仍会报错，包括非法 intent，且不会覆盖数据库和已有备份。派生重建和结构验证同时覆盖热区与归档区。数据库 `format_version` 为 1，缺少 v1.1/v1.2 新字段的旧文件按规格补齐，无 intent 时视为未标注；MCP 工具集契约为 1.2.0，仍提供 14 个工具。含标注的数据库应使用 v1.2 或更新版本读写，旧程序不保证保留新字段。

## 审计与恢复

审计位于 **DB 顶层 audit 数组**。每次真实摘要、修正、新占位创建均记录 Unix 秒 ts、操作、timeline、区间和节点；归档、幂等映射也记日志。Storage 注入不递减时间并拒绝改写/删除旧审计。占位复用不是创建，不重复写创建审计。

按审计中的 from/to 调用 `node --node FROM` 和 `context --node TO` 可以取回原文，旧摘要也能按 ID 查询。如果数据库文件损坏，先停止使用它的所有 CLI/MCP 进程，确认 `.bak` 存在，保存损坏文件后再恢复。下面给出两种 shell 的示例，路径需换成你实际使用的数据库。

Windows PowerShell：

```powershell
$timelineDb = Join-Path $env:USERPROFILE 'timeline-data\conversation.timeline.json'
if (-not (Test-Path -LiteralPath "$timelineDb.bak")) { throw '未找到数据库备份' }
$savedBroken = "$timelineDb.corrupt-$(Get-Date -Format yyyyMMdd-HHmmss)"
Copy-Item -LiteralPath $timelineDb -Destination $savedBroken -ErrorAction Stop
Copy-Item -LiteralPath "$timelineDb.bak" -Destination $timelineDb -ErrorAction Stop
tl --db $timelineDb branches
```

Linux/macOS（bash 或 zsh）：

```sh
timeline_db="$HOME/.local/share/timeline/conversation.timeline.json"
if [ -f "$timeline_db.bak" ]; then
  cp -p "$timeline_db" "$timeline_db.corrupt-$(date +%Y%m%d-%H%M%S)" &&
  cp -p "$timeline_db.bak" "$timeline_db" &&
  tl --db "$timeline_db" branches
else
  printf '%s\n' '未找到数据库备份' >&2
fi
```

恢复回到上一次成功变更前的快照；首次写入没有旧快照，因此没有 .bak。程序不会自动覆盖损坏文件。

## MCP stdio 接入

让客户端启动 `tl-mcp`，并通过 stdin/stdout 与它通信即可。下面是通用启动字段，外层结构按客户端要求填写。将 `YOUR_NAME` 和安装目录换成实际路径，并先创建数据库的父目录；程序不会自动创建目录。

Windows：

```json
{
  "command": "C:\\Users\\YOUR_NAME\\tools\\timeline\\tl-mcp.exe",
  "args": ["--db", "C:\\Users\\YOUR_NAME\\timeline-data\\conversation.timeline.json"],
  "env": {}
}
```

Linux：

```json
{
  "command": "/home/YOUR_NAME/.local/bin/timeline/tl-mcp",
  "args": ["--db", "/home/YOUR_NAME/.local/share/timeline/conversation.timeline.json"],
  "env": {}
}
```

macOS（Apple Silicon 和 Intel 配置形式相同，使用各自的包）：

```json
{
  "command": "/Users/YOUR_NAME/.local/bin/timeline/tl-mcp",
  "args": ["--db", "/Users/YOUR_NAME/.local/share/timeline/conversation.timeline.json"],
  "env": {}
}
```

图形客户端不一定继承终端的 PATH，因此推荐写绝对路径。普通 JSON 字符串中的 `~`、`$HOME`、`%USERPROFILE%` 不应当作已展开的路径；如客户端支持变量替换，按它的说明配置。也可以省略 `--db`，在客户端传入的环境中设置 `TIMELINE_DB`，或使用用户目录下的默认数据库。

### 协议与客户端集成

服务每行收发一个 UTF-8 JSON-RPC 2.0 消息，stdout 只输出协议数据。单帧上限为 8 MiB；超限或非法 UTF-8 会返回协议错误，然后继续处理后续帧。握手和首次创建的消息示例如下；将 `tl0` 换成 create 实际返回的 ID：

```jsonl
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"example","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"timeline_create","arguments":{},"idempotency_key":"new-conversation-001"}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"timeline_append","arguments":{"timeline_id":"tl0","role":"user","text":"你好","idempotency_key":"message-001"}}}
```

支持 ping。初始化报告服务名 timeline，服务版本与 `Cargo.toml` 的包版本一致，当前为 1.2.0；支持协商 2024-11-05、2025-03-26、2025-06-18、2025-11-25，未知版本返回 2025-11-25 供客户端决定是否继续，不承诺未来协议功能。旧两个版本支持 batch 串行处理；2025-06-18 及以后只接受单请求。遵循 [MCP 生命周期](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)、[stdio 传输](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)与[旧版基础协议](https://modelcontextprotocol.io/specification/2025-03-26/basic)。

所有版本的成功结果均在 `result.content[0].text` 中保存工具 JSON 字符串；2025-06-18 及以后的对象结果同时返回 structuredContent。幂等键可在 arguments 或 params 中，两处同时提供须一致。领域错误 code=-32000，协议错误使用对应标准错误码；通知不回应也不执行变更工具。预算不可行返回 `error` 对象 `{"code":-32000,"message":"budget infeasible","data":{"required_tokens":65,"max_tokens":1}}`（此数值对应上文反例）。

更多信息：[技术事实](TECH.md)、[文档索引](docs/README.md)、[环境说明](docs/environments.md)、[发布指南](docs/releases.md)、[机器可读检查命令](.project-governance.json)。
