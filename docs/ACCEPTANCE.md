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

后续提交的四平台结果和产物见 [Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml)；本表保留已确认的运行记录，Intel 检查结果尚待补充。Intel runner 的可用性与包的选择见 [运行环境](environments.md#ci-平台与兼容范围)。

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
