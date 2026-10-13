# 运行环境与数据库路径

CLI/MCP 是本机 Rust 二进制，不依赖外部服务，不监听端口，不访问网络。下载 Cargo 依赖属于构建阶段。

| 环境变量 | 用途 | 缺省行为 |
|---|---|---|
| TIMELINE_DB | CLI/MCP 共用的 JSON 数据库路径 | 未设置或空值则取用户目录的 `.timeline.json` |
| USERPROFILE | Windows 的用户目录，由系统提供 | 缺失时回退 HOME；Unix 不读取 |
| HOME | Unix 用户目录，Windows 的回退目录 | 没有可用目录时须提供显式 --db 或非空 TIMELINE_DB |

优先级为 **`--db PATH` > 非空 `TIMELINE_DB` > 默认用户目录**。CLI 与 tl-mcp 使用同一解析函数。父目录须已存在，路径支持 Unicode/空格；空的显式 --db 报错。应用不自动读取 `.env`，`.env.example` 仅列变量名，勿用空值覆盖系统 USERPROFILE/HOME。

PowerShell 示例（目录应已存在）：

```powershell
$env:TIMELINE_DB = 'C:\timeline-data\conversation.timeline.json'
tl create
tl-mcp
```

`<db>` 为全量快照，`<db>.lock` 为稳定跨进程锁，`<db>.bak` 为上一快照。事务使用同目录 `.timeline-*` 临时文件；正常完成/错误时清理，异常进程终止留下的临时文件不参与加载。每次成功变更全量 checkpoint，未启用增量日志或后台定时器。

数据库与备份都含对话、摘要、审计和幂等结果，未加密；应放在可信的本地目录，不提交真实内容。Unix 新文件为 0600，Windows 创建时使用受保护 owner-only DACL。

本次实际验证环境：Windows、Rust/Cargo 1.97.1、Python 3.13.14（仅验收/治理脚本使用）。Unix 有相应回归但本次未执行；Cargo 声明的最低 Rust 1.85 未单独验证。

恢复前停止所有使用此数据库的进程、保留损坏文件，再从 .bak 显式恢复；详见 [README](../README.md#审计与恢复)。程序不会静默覆盖损坏源数据。

机器清单见 [`.project-governance.json`](../.project-governance.json)，技术边界见 [`TECH.md`](../TECH.md)。
