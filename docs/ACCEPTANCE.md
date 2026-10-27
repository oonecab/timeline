# Timeline 验收记录

本文分开记录跨平台 CI 与本地 §13 验收。CI 链接对应固定提交，本地耗时对应当时的 Windows 环境，不能用其中一项替代另一项。

## 跨平台 CI

[CI 工作流](../.github/workflows/ci.yml) 在 push 和 pull request 时运行。矩阵中的每个平台都执行 `cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --check`，检查通过后构建 Release 并上传二进制。Intel Mac 与其他平台使用同一个 test job，也会完整执行这些检查。

| Runner | 已记录的证据 | 产物 |
|---|---|---|
| `windows-latest` | 提交 `2db1761` 的 CI 通过 | `timeline-windows-latest` |
| `ubuntu-latest` | 提交 `2db1761` 的 CI 通过 | `timeline-ubuntu-latest` |
| `macos-latest`（Apple Silicon） | 提交 `2db1761` 的 CI 通过 | `timeline-macos-latest` |
| `macos-15-intel`（Intel） | 提交 `2db1761` 的原生完整 CI 通过 | `timeline-macos-15-intel` |

四平台结果来自 [Actions 运行 38409120427](https://github.com/oonecab/timeline/actions/runs/38409120427)，对应[提交 `2db1761`](https://github.com/oonecab/timeline/commit/2db17614f84cf27873990e4bf3959b00273d418a)。Intel Mac 与另外三项均完成了测试、Clippy、格式检查、Release 构建和对应产物上传。其中 Ubuntu 和两种 Mac 执行了 Unix `0600` 权限回归，Windows 执行了受保护 DACL 回归；这些结果覆盖该次 runner 环境，不代表所有系统版本与 CPU 架构都已验证。此前三平台记录仍可查看 [运行 38405720276](https://github.com/oonecab/timeline/actions/runs/38405720276)。

后续提交的四平台结果和产物见 [Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml)；本表保留已确认的运行记录。Intel runner 的可用性与包的选择见 [运行环境](environments.md#ci-平台与兼容范围)。

## 本地 §13 验收：2026-10-13

验收日期：2026-10-13（Asia/Shanghai）。环境：Windows，Rust 1.97.1，Cargo 1.97.1，Python 3.13.14。构建依据为 TIMELINE-SPEC v1.1.2，数据库格式保持 1，MCP 工具集版本保持 1.1.0。以下保留当时的测试结果与耗时。

纯 Rust ADT、JSON 持久化、`tl` CLI 和 `tl-mcp` stdio 服务已适配新契约。**74 个测试通过，0 失败、0 忽略。** 相较前一版新增 12 项测试，并更新 §9 公理中的归档与预算断言：被摘要覆盖的独有原文搬运、冷热引用重建，以及预算不可行的精确错误载荷均通过。其余边界说明见 [修正依据与实现解释](SPEC-NOTES.md)。

### 统一执行命令

Windows PowerShell：

```powershell
python scripts/acceptance.py
```

Linux/macOS：

```sh
python3 scripts/acceptance.py
```

该脚本按序执行下列四个命令。所有数据库测试使用临时目录，不写用户默认数据库。表中耗时来自上述 Windows 历史记录。

| 命令 | 退出码 | 记录耗时 |
|---|---:|---:|
| `cargo fmt --all -- --check` | 0 | 0.130s |
| `cargo clippy --all-targets --locked -- -D warnings` | 0 | 0.816s |
| `cargo test --all-targets --locked` | 0 | 17.909s |
| `cargo build --release --bins --locked` | 0 | 3.021s |

脚本将完整输出写入 `target/acceptance/commands.log`，将机器可读结果写入 `target/acceptance/results.json`。这些文件不会提交到仓库，再次运行会覆盖它们，当前本地日志可能已经对应后续运行。Release 产物位于 `target/release/`：Windows 为 `tl.exe`、`tl-mcp.exe`，Linux/macOS 为 `tl`、`tl-mcp`。

### §13 逐项结果

| # | 原清单验收项 | 结果与证据 |
|---:|---|---|
| 1 | §9 的 15 个测试（含 fuzz）无失败 | **通过**。`tests/axioms.rs` 15/15，包含 v1.1.2 指定不可行反例及精确载荷、v1.1.1 非空归档。另有 20 项边界测试及 9 项本版契约测试。 |
| 2 | 随机操作后检查 10 条不变量与前缀和 | **通过**。1200 次混合操作检查源字段、父子关系、唯一根、分支、区间、前缀和、水位与版本；新增 128 个随机双阶段归档场景检查冷热合并图、源节点完整保留、摘要留热、引用计数和重启一致性。`tokens()` 直接读取字段。 |
| 3 | CLI 全命令与 JSON 形状 | **通过**。真实进程调用全部 14 个命令，包含扩展摘要参数、切换/删除/归档、非法输入、JSON stderr 与非零退出码。 |
| 4 | MCP 握手、14 工具、全部调用与错误 | **通过**。真实 stdio 进程测试 initialize、通知、ping、tools/list（精确 14 个）、全部 tools/call 和业务错误 -32000；另覆盖版本协商、旧协议 batch、超长帧与非法 UTF-8 后恢复。 |
| 5 | 预算上界 fuzz 1000 次 | **通过 v1.1.2 契约**。1000 个可行随机链满足上界；另 1000 个带 pin/摘要的随机场景独立判断可行性。规格反例精确返回 `budget infeasible` 与 `{required_tokens:65,max_tokens:1}`；核心、CLI、MCP 均验证，失败不写数据库、备份、审计或幂等结果。保留 §5.3 的 pin 本身超预算例外。 |
| 6 | 摘要及占位创建审计、原文可回溯 | **通过**。每次实际摘要/占位创建均记录 `DB.audit` 的 op/from/to/node；原文 context 保留。Storage 注入非递减时间戳，拒绝删除/修改已有日志。归档和幂等结果也有审计。 |
| 7 | README 声明 RESERVE、估计器、审计位置 | **通过**。README 声明 `RESERVE=32`、`max(1, ceil(Unicode 标量数/4))` 和 `DB.audit`；极限 ID/条数宽度的占位成本也有测试。 |
| 8 | 水位不变量 7/8 与优化等价性 | **水位通过；可选优化未启用**。1000 次随机终点摘要检查水位不回退，1200 次混合操作检查完整性；已覆盖取消 pin 保护和批量占位推进。开/关参数的输出一致，但两者均运行完整扫描，不能解读为已实现扫描优化。 |
| 9 | 归档三条件与共享节点保护 | **通过 v1.1.1 契约**。缺水位、有 pin、未解决占位与当前分支仍拒绝；被摘要覆盖的独有原文实际从 nodes 移至 archive，共享节点与摘要留热，返回实际搬运 ID。验证冷热 parent/children、水位、原文上下文及重启重建，二次归档不重复返回旧冷节点。CLI/MCP 均实测非空搬运。 |
| 10 | 同 key 无重复节点、结果相同 | **通过**。append/summarize 重放返回同一 ID；跨进程重启、不同 timeline 作用域、碰撞安全键编码、7 天过期、8 个并发同 key 均验证。 |
| 11 | 摘要版本链、旧版可取、下游 stale | **通过**。新版本递增且 supersedes 指向旧版；拒绝版本分叉与错误区间；旧版内容不变，多级 derived_from 级联 stale，包括归档中的依赖重建。 |
| 12 | README 声明派生字段、水位、checkpoint | **通过**。派生数据列表、取消 pin 保护、水位语义和每次变更全量快照均已记录；可选增量日志未实现，未声称已实现。 |
| 13 | `tl-mcp --db` / `TIMELINE_DB` 路径覆盖 | **通过**。真实 CLI/MCP 进程分别验证 `--db > TIMELINE_DB > 用户目录/.timeline.json`，以及 CLI 默认最早 timeline / 显式选择。 |

### 测试分组

| 分组 | 通过 | 失败 / 忽略 |
|---|---:|---:|
| 库单元测试 | 1 | 0 / 0 |
| `tests/axioms.rs` | 15 | 0 / 0 |
| `tests/adversarial.rs` | 20 | 0 / 0 |
| `tests/interfaces.rs` | 14 | 0 / 0 |
| `tests/limits.rs` | 4 | 0 / 0 |
| `tests/spec_v112.rs` | 9 | 0 / 0 |
| `tests/storage.rs` | 11 | 0 / 0 |
| **合计** | **74** | **0 / 0** |

存储额外验证了 72 次并发事务、8 个独立 CLI 进程、原子备份失败回滚、JSON/源字段损坏拒写、旧格式兼容和任意类型损坏的派生字段重建。这份本地记录包含 Windows 的 DB/备份/锁文件受保护 DACL 测试；Unix `0600` 测试由条件编译选择，不会在 Windows 上运行。Linux/macOS 的执行证据见上方[跨平台 CI](#跨平台-ci)，不能仅凭本地“0 忽略”推断跨平台通过。

### 验收时的实现边界与文档影响

没有实现 merge、网络同步、向量检索、自动摘要策略或 v1.2 显式恢复操作。归档节点现在可实际搬运，冷热只读查询透明；为保持分支 head 位于热区，直接 fork 冷节点明确报错，可从热摘要 fork 后继续追加并读取其冷祖先。水位扫描与增量 checkpoint 两项可选优化仍未启用。

适配 v1.1.2 时，README、SPEC-NOTES、TECH 和治理清单移除了“归档恒为空”“不可行错误偏离规格”的过时结论，记录了 v1.1.1/v1.1.2 的修复、错误载荷和冷热读取边界。该次变更没有新增依赖、迁移数据库格式、改变环境变量或改写规格。

项目采用 lean / generic 治理。该次普通治理校验通过，提示人工维护的 README 需核对，随后完成核对。治理生成器的 `--strict` 将这条提示当作失败并返回 1，因此该记录不包含严格模式通过的结论；治理校验不属于 §13 的四项 Rust 验收命令。后续预览未发现受管文件漂移。

## 跨平台文档更新范围

增加 Intel Mac CI 矩阵项时，同步补充了各平台的下载、启动、数据库路径、MCP 配置及恢复说明，并将历史本地验收与持续 CI 证据分开记录。此次调整不改变协议、业务规则、数据库格式、环境变量或依赖；现有回归测试继续由各平台 CI 执行。

## Tag 发布工作流验证：2026-10-13

新增 [Release 工作流](../.github/workflows/release.yml) 后，本地重新运行了 `.project-governance.json` 登记的 fetch、fmt、clippy、test 和 release build，五项退出码均为 0；Windows 测试 74 通过、0 失败、0 忽略。证据保存在未提交的 `target/prepush-release-validation/`。

本地还检查了 YAML 中的 tag 触发、四平台矩阵、发布依赖与权限、四个完整资产路径，并确认 Release 与现有 CI 的检查和构建步骤一致。执行了工作流中的 Windows 打包命令，确认 ZIP 根目录只有 `tl.exe`、`tl-mcp.exe`，解压后两者的 `--help` 均成功；Unix 打包命令通过 Bash 语法检查。未创建 tag，因此没有执行远端 Release 发布；Linux/macOS 的本次打包和上传仍需实际 tag 运行验证。actionlint 未能运行：本机未安装，下载工具时 GitHub TLS 连接失败；以上 YAML 结构及 shell 检查是本次替代验证。

下载与发布说明已同步到 README、环境说明、文档索引和 [发布指南](releases.md)，技术事实已同步到 TECH。此次仅增加 GitHub Actions 发布流程，未改变应用依赖、运行时环境变量、数据库格式或业务规则，因此无需更新应用环境清单及业务规则测试。

## draw.io 导出验收：2026-10-15

在 Windows 本地执行 `python scripts/acceptance.py`，**85 个测试通过，0 失败、0 忽略**，其中新增 `tests/drawio.rs` 的 11 项回归。此前 74 项核心、存储与 CLI/MCP 测试全部继续通过。当前运行日志及机器可读结果在 `target/acceptance/commands.log` 和 `target/acceptance/results.json`。

| 命令 | 退出码 | 本次耗时 |
|---|---:|---:|
| `cargo fmt --all -- --check` | 0 | 0.153s |
| `cargo clippy --all-targets --locked -- -D warnings` | 0 | 0.251s |
| `cargo test --all-targets --locked` | 0 | 17.436s |
| `cargo build --release --bins --locked` | 0 | 3.489s |

另外执行了治理清单中的 `cargo fetch --locked` 和 `cargo build --release --locked`，退出码均为 0。

| 导出验收项 | 结果与证据 |
|---|---|
| 原生节点、连线和完整数据 | **通过**。独立 XML 解析器校验节点 ID 唯一、父子边准确、关系目标存在、完整原文与摘要版本数据可恢复；共享前缀只出现一次。 |
| 截图风格的横向布局 | **通过**。当前路径水平排列，分支分行，节点不重叠；使用浅蓝圆角节点、绿色当前 head 和灰色折线箭头。同一状态的输出确定，不受 HashMap 迭代顺序影响。 |
| 冷热历史、摘要与占位 | **通过**。归档原文和热摘要全部导出，跨冷热父子关系保留；摘要覆盖、版本替代、依赖分别标注且放入独立图层；不创建或回填占位。 |
| 文本与长历史边界 | **通过**。中文、emoji、HTML 字面量、引号、控制字符和 XML 非法字符可安全输出；长消息完整保留，展示预览有限；6000 条消息的线性历史导出通过。 |
| 文件只读与手工修改 | **通过**。真实 CLI 进程验证导出前后数据库和备份逐字节不变；修改图文件后，查询得到的原始消息不变。已有输出文件拒绝覆盖，保留人工修改。 |
| 参数、错误和并发 | **通过**。验证默认最早时间线、显式选择、环境变量、缺失时间线、损坏数据库、不支持的格式、无效输出目录、拒绝幂等键，以及两个进程竞争同一输出时仅一个成功。 |
| 官方图形渲染与编辑保存 | **通过**。使用 draw.io 官方 `viewer-static.min.js` 中的 Graph / mxCodec，在无网络访问的本地 Chromium 页面解码实际 CLI 文件；8 个节点与 9 条边正常载入，节点可移动、可编辑。移动节点、修改标题后重新编码并解码，位置、标题和完整原文元数据均保留；开启摘要关联图层后连线可见，页面无 JavaScript 错误。 |

官方渲染检查使用合成对话，示例保存在 [examples/timeline.drawio](../examples/timeline.drawio)。本地截图及修改后的 XML 在未提交的 `target/drawio-smoke/`；官方脚本仅用于本次验证，不随应用发布，也没有引入网页服务。这项检查覆盖官方渲染和文件编解码，没有声称执行完整 draw.io 桌面应用的人工操作。Unix 专用的符号链接目标保护测试未在本机运行，需由 Linux/macOS CI 验证；上方历史 CI 结果不代表此次导出变更已通过远端 CI。

文档影响：新增 [导出说明](drawio.md)，同步 README、文档索引、TECH 与治理清单中的架构事实，并提供可打开的合成示例。`roxmltree` 仅用于测试；运行时依赖、存储实现与格式、14 个 MCP 工具、环境变量和端口均保持不变，因此无需修改构建规格、存储规则、环境清单或新增 ADR。

## ASCII 终端图验收：2026-10-15

增加 `export --format ascii` 后，本地重新执行 `python scripts/acceptance.py`，**94 个测试通过，0 失败、0 忽略**。其中新增 `tests/ascii.rs` 的 9 项回归，原有 11 项 draw.io 测试与 74 项核心、存储及接口测试全部通过。`cargo fmt --all -- --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --all-targets --locked`、`cargo build --release --bins --locked` 退出码均为 0；完整日志和实际耗时保存在 `target/acceptance/`。治理清单中的 `cargo fetch --locked` 与 `cargo build --release --locked` 也执行成功。

| 验收项 | 结果与证据 |
|---|---|
| 终端分支结构 | **通过**。逐字比对分叉图，校验共享前缀仅出现一次、当前路径优先、多分支 head 同节点显示与 HashMap 顺序无关。空时间线和 6000 条消息的长链通过，连续历史不增加缩进。 |
| 冷热、摘要与只读性 | **通过**。完整列出冷热节点、版本替代、覆盖区间、依赖、STALE 和占位；ASCII 与 draw.io 的节点及关联计数相同。渲染前后数据库对象不变，不填充占位。 |
| 文本与控制字符 | **通过**。保留中文与 emoji，限制展示预览，转义换行、回车、制表符、终端控制序列及方向控制符；原文保持不变。 |
| stdout 与文件契约 | **通过**。stdout 直接输出文本图，`--output` 保存相同 UTF-8 内容并返回 JSON；拒绝覆盖手改文件、拒绝以数据库为输出，数据库和备份逐字节不变。错误只写 stderr JSON，draw.io 的默认格式及必填输出路径保持不变。 |
| 管道提前关闭 | **通过**。实际 CLI 子进程的 stdout 读取端被关闭时正常退出，无 panic 或额外 stderr。 |
| PowerShell 5.1 / 7 | **通过**。在 Windows PowerShell `5.1.26100.7920` 和 PowerShell `7.6.6` 中调用 Release 二进制，显式使用 UTF-8 解码，验证中文 stdout 与 `--output` 文件一致、`Select-Object -First 5` 管道有效、源文件 SHA256 不变。 |
| Git Bash | **通过**。Git Bash `5.3.15` 调用 Windows Release 二进制，验证 stdout 重定向、`head -n 5` 管道，以及输出字节与 PowerShell 文件导出一致。此项不代表已运行 Linux/macOS 原生二进制。 |
| draw.io 兼容 | **通过**。提取共用只读拓扑后重新导出相同合成数据，XML 与上次 [draw.io 示例](../examples/timeline.drawio) 的 SHA256 相同。 |

shell 验证脚本、结果与文件位于未提交的 `target/ascii-smoke/`。可直接查看 [ASCII 合成示例](../examples/timeline.ascii.txt)。本次没有运行远端 CI，上方历史跨平台记录不作为本次变更的执行证据。

文档影响：新增 [终端图说明](ascii.md)，更新 README、文档索引、draw.io 格式选项说明、TECH 与治理清单，明确只有 ASCII 无输出路径时直接打印文本。没有新增依赖，没有修改存储实现、MCP、业务规则、环境变量或端口，因此无需修改规格、环境清单或新增 ADR。

## v1.1.1 发布前验收：2026-10-15

将 draw.io / ASCII 导出纳入此次发布，并把 Cargo 包版本升为 `1.1.1`。两个程序的 `--version` 和 MCP `serverInfo.version` 统一读取 Cargo 包版本；MCP 工具集契约与数据库格式保持不变。版本行为已同步到 README、TECH 和发布指南。

重新执行治理清单登记的 `cargo fetch --locked`、格式检查、Clippy、全目标测试及 Release 构建，五项退出码均为 0；**94 个测试通过，0 失败、0 忽略**。实测 `tl --version` 为 `tl 1.1.1`、`tl-mcp --version` 为 `tl-mcp 1.1.1`，使用独立临时数据库路径的 MCP 初始化返回服务版本 `1.1.1`。该初始化未创建数据库文件。

本地证据保存在未提交的 `target/prepush-v1.1.1-validation/`。上述记录为发布前的 Windows 本地结果；tag 推送后的四平台检查、打包及发布结果以 [Release Actions](https://github.com/oonecab/timeline/actions/workflows/release.yml) 为准。

## v1.2.0 意图标注与 PR #1 修复验收：2026-10-27

本次验收针对 [PR #1](https://github.com/oonecab/timeline/pull/1) 的节点意图标注及其审查修复。Windows 本地执行治理清单登记的五项检查，全部成功；**102 个测试通过，0 失败、0 忽略**。四项新增回归中，摘要去重、冷热损坏保护、根与占位标注校验在修复前均能复现失败，修复后通过；原有测试保持启用。

| 命令 | 结果 |
|---|---|
| `cargo fetch --locked` | 退出码 0 |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | 退出码 0 |
| `cargo test --all-targets --locked` | 退出码 0，102 项通过 |
| `cargo build --release --locked` | 退出码 0 |

`tests/spec_v120.rs` 共 8 项测试，覆盖旧数据库默认无标注、标注不可变、参数校验、上下文输出，以及以下修复边界：

- 摘要去重包含完整 intent，区分无标注、无备注、空备注及不同标签/备注；保存后重建索引仍复用相同请求的节点。
- 热区和冷区均拒绝空标签、缺失标签及错误字段类型；失败发生在事务执行前，数据库和已有备份逐字节不变。
- 根与机械占位不能带标注；直接创建带标注的占位也会失败，且不消耗节点或修改状态。
- 存储往返保留完整 intent，context/budget 仅输出 intent_tag，不输出 note，也不将 intent_tag 写入 message。

另用实际 Release 二进制和独立临时数据库检查 CLI 的 append/node/context/budget，确认单独传 `--intent-note` 会报错且数据库未变；通过真实 stdio MCP 进程验证 initialize/tools-list/summarize/context、标注 schema 与 14 个工具。两个程序及 MCP 握手均报告版本 `1.2.0`。日志、逐项退出码和冒烟结果保存在未提交的 `target/pr-1-fix-validation/`。

文档影响：同步 TIMELINE-SPEC v1.2.0、README、TECH、治理清单、SPEC-NOTES 和文档索引，补全输入输出、不可变、去重与损坏数据保护规则；发布指南更新版本示例，导出说明改为引用当前规格。没有新增依赖，没有修改 workflow、环境变量、端口或数据库格式版本，因此环境清单、`.env.example` 和技术栈变更索引无需更新，也无需新增 ADR。上述结果为 Windows 本地证据，PR 当前提交的四平台结果以 [CI Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml) 为准；本次未打 tag 或发布 Release。
