# Timeline — 技术事实唯一来源

<!-- project-governance:managed:start -->
## 项目目的

给 LLM 对话提供持久化分支、可逆摘要和 MCP 接口

## 项目分类

- 项目类型：`generic`
- 治理级别：`lean`

## 技术栈基线

### 语言

- Rust 2024 edition（Cargo 声明 rust-version 1.85；Windows 本地验收工具链 1.97.1，CI 使用 stable）

### 框架与核心库

- serde 1 / serde_json 1：JSON 数据模型与协议
- clap 4：CLI 参数
- fs2 0.4：跨进程文件锁
- tempfile 3：同目录原子快照
- windows-sys 0.61：Windows 创建时的私有 DACL

### 运行时与工具链

- 使用 Rust / Cargo 构建；发布产物是本机原生二进制，运行时无需 Rust 或外部服务。

## 源码边界

- src
- tests

## 架构事实

- 纯内存 Core → JSON Storage → CLI / stdio MCP；Core 无 I/O、时钟或随机数。
- 单 JSON 文件 format_version=1；稳定 sidecar 文件锁串行化事务；临时快照 fsync 后原子替换并保留 .bak。
- MCP 提供 14 个工具，按行处理 stdio JSON-RPC；应用运行期间不访问网络、不监听端口。
- 源数据持久化，派生字段和索引读取时重建；每次成功变更全量 checkpoint，未启用可选增量日志或水位扫描优化。
- 构建依据为 TIMELINE-SPEC v1.1.2；归档将独有原文搬入同文件冷区，摘要保留在热区，原文及 parent/children 不变；只读查询跨冷热区。预算不可行返回 budget infeasible 与 required_tokens/max_tokens；历史修正与边界解释见 docs/SPEC-NOTES.md。

## 项目类型约束

- 明确领域边界，并在 TECH.md 中记录项目专属架构事实。

## 项目专属规则

- TIMELINE-SPEC.md 是权威构建规格，发现内部冲突时必须在 docs/SPEC-NOTES.md 公开说明。
- 单条消息最多 1 MiB UTF-8；上下文输出最多 100000 条消息。
- 不得把未启用的可选优化或未执行的平台测试报告为已经实现或验证。
<!-- project-governance:managed:end -->

## 已确认的技术细节

在此记录模块边界、数据流、技术兼容性、跨系统数值语义，以及规则权威文档的链接。工作流程规范不得写入本文件。

- `src/lib.rs`：纯 ADT、全局确定性计数器、操作契约、源结构验证与派生索引重建；Timeline.nodes/branches 为 HashMap，Database.timelines 为 BTreeMap。CLI 默认 timeline 按 ID 数字选最早创建者。
- `src/storage.rs`：路径、稳定 sidecar 文件锁、事务、Unix 秒时间、7 天幂等结果、全量原子快照及上一版本 `.bak`。持久化计数器载入 Database 后由核心在事务内分配；核心不读取时钟或产生随机 ID。
- `src/service.rs`：14 项工具的参数校验与分发；`src/bin/tl.rs`、`src/bin/tl-mcp.rs` 分别提供 CLI 与逐行 stdio JSON-RPC。
- `tests/`：公理与随机场景、CLI/MCP 全流程、损坏/旧格式、幂等、并发及平台权限；存储回归仅操作临时目录。`scripts/acceptance.py` 一键执行已登记检查，日志写入 `target/acceptance/`。
- `.github/workflows/ci.yml`：GitHub Actions 在 push/pull_request 时使用 stable Rust，分别在 `ubuntu-latest`（x64）、`macos-latest`（arm64）、`macos-15-intel`（x64）和 `windows-latest`（x64）执行 test、clippy、fmt，再构建 Release。四个 job 执行相同检查，包括 Intel Mac 上的原生测试。产物命名为 `timeline-${{ matrix.os }}`，只包含对应 runner 的 `tl`/`tl-mcp`（Windows 为 `.exe`）。
- `.github/workflows/release.yml`：推送任意 tag 时，在上述四个平台重新执行相同检查与构建；四个平台全部成功后，由独立 job 发布该 tag 的 GitHub Release。Windows 包为 ZIP，Linux/macOS 包为保留执行权限的 tar.gz，每包根目录仅含两个二进制。构建 job 只有 `contents: read`，发布 job 通过 GitHub 自动提供的令牌获得 `contents: write`；未增加应用环境变量或外部运行时服务。发布步骤和下载方式见 [发布说明](docs/releases.md)。
- CI 二进制需要匹配使用者的系统及 CPU 架构；Intel Mac 包不是通用 macOS 包，也不保证所有旧版 macOS 都兼容。`macos-15-intel` 的可用期受 GitHub 镜像弃用安排约束，不能作为永久分发渠道；平台选择及官方公告见 [环境说明](docs/environments.md)，实际测试结果见 [验收记录](docs/ACCEPTANCE.md)。
- 数据格式保持 v1，MCP 工具集版本保持 1.1.0；新元数据缺失按规格补齐，损坏的 prefix_tokens/stale 丢弃后重建；parent/children/message/branches 等源数据损坏拒绝读取及覆盖。
- 归档把独有无 pin 原文从 nodes 搬到 archive；摘要留热，允许其 parent 指向冷区，不改写 parent/children/message。核心节点、路径、context、tokens 及索引重建读取两区；fork 必须以热节点为起点，显式冷区恢复预留给 v1.2。
- 预算不可行错误的消息为 budget infeasible，数据为最低布局总成本 required_tokens 和输入 max_tokens；CLI 在 error/data 返回，MCP 在 JSON-RPC error.message/data 返回。失败不提交任何变更。
- 派生状态包括 prefix_tokens、covers、真实摘要语义覆盖、占位复用、真实摘要区间、摘要参数索引、版本后继、依赖反向关系、水位路径和 stale；refcount 按查询计算。完整清单见 [README](README.md)。
- 新建快照、备份、锁在 Windows 使用创建时受保护 DACL `D:P(A;;FA;;;OW)`，Linux/macOS 使用 `0600`；权限回归在对应平台条件编译并由 CI 执行，已确认的运行证据见 [验收记录](docs/ACCEPTANCE.md)。Windows 通过 tempfile 的 MoveFileExW 覆盖，Linux/macOS 通过 rename 并同步父目录，不先删除目标。
- 应用只读取 TIMELINE_DB/USERPROFILE/HOME，不自动加载 `.env`；无外部服务、网络访问或监听端口。环境权威清单见 [docs/environments.md](docs/environments.md)。
- [TIMELINE-SPEC.md](TIMELINE-SPEC.md) v1.1.2 是构建权威；已解决的预算与归档冲突，以及不可变回填、unpin、占位和水位边界解释见 [docs/SPEC-NOTES.md](docs/SPEC-NOTES.md)。未启用增量日志；水位优化的两个参数值目前执行相同全路径扫描。

## 技术栈变更索引（只追加）

“技术栈变更”至少包括语言、运行时、框架、核心库、构建工具、数据库或中间件的新增、移除、替换、主版本升级，以及兼容基线变化。普通补丁升级仅在影响行为、兼容性或安全基线时记录。

| 日期 | 变更前 → 变更后 | 科学/工程理由 | 兼容、迁移与回滚影响 | 问题单 / ADR |
|---|---|---|---|---|
| 2026-10-13 | 未实现 → Rust 2024、Cargo、serde/serde_json、clap、fs2、tempfile、Windows 下 windows-sys | 按规格实现纯核心、单文件事务与 CLI/MCP；原子替换、锁和私有权限保护快照 | 格式保持 v1，新增字段兼容旧文件；无外部数据库迁移或部署。`.bak` 保留旧快照。Rust 最低声明 1.85，实际验证 1.97.1 | [构建规格](TIMELINE-SPEC.md) / [规格说明](docs/SPEC-NOTES.md) |

> 详细论证写入问题单或 ADR；本表保存可追踪索引，不得覆盖历史记录。
