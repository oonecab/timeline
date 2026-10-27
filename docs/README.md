# 文档职责索引

<!-- project-governance:managed:start -->
每类事实只能有一个权威来源：

- `../TECH.md`：技术栈、架构、路径、依赖、兼容边界与技术栈变更索引。
- `../AGENTS.md`：短小的 Agent 入口和不可协商红线。
- `../.project-governance.json`：结构化项目设置与已登记命令。
- `governance/`：工作流、测试、安全、变更控制、发布和风险规范。
- `environments.md`：不含秘密的环境变量名、服务和端口清单。
- `rules/`：公式、统计口径、舍入、单位、币种、时间边界等业务/计算规则的权威文档。
- `decisions/`：受控级别下难以回退的技术与架构决策。

禁止在多个文件中手工维护同一事实。其他文档必须链接到权威来源。代码变更完成前必须评估并同步受影响文档。
<!-- project-governance:managed:end -->

## 项目文档

初次使用建议先看 README，按 Windows、Linux 或 Mac 架构找到对应的下载与 CLI/MCP 配置说明。需要修改代码时，再阅读技术说明、构建规格与测试记录。

| 想了解的内容 | 文档 |
|---|---|
| 下载与启动、CLI/MCP 配置、意图标注、预算、存储与恢复 | [README](../README.md) |
| 导出可手动编辑的横向节点图、图层含义与只读边界 | [draw.io 导出](drawio.md) |
| 在 Bash / PowerShell 查看 ASCII 分支树、管道与文本文件输出 | [终端图](ascii.md) |
| 用 tag 发布四平台二进制、发布权限与重跑规则 | [发布指南](releases.md) |
| 各系统的路径与环境变量、执行权限、CI 平台兼容范围 | [运行环境](environments.md) |
| 架构、模块、依赖及技术栈变更 | [TECH.md](../TECH.md) |
| 数据模型、操作契约、§9 测试映射与 §13 验收清单 | [TIMELINE-SPEC.md](../TIMELINE-SPEC.md) |
| 预算与归档的历史修正、v1.2 标注边界与回归设计 | [SPEC-NOTES.md](SPEC-NOTES.md) |
| Windows 本地历史验收、持续 CI 与各平台的执行证据 | [ACCEPTANCE.md](ACCEPTANCE.md) |
| Agent 如何在仓库中协作 | [AGENTS.md](../AGENTS.md) |

[tests](../tests) 保存可执行回归，[acceptance.py](../scripts/acceptance.py) 可一次运行本地格式、静态检查、测试和 Release 构建，日志写入 `target/acceptance/`。日常远端检查由 [CI 工作流](../.github/workflows/ci.yml) 执行，结果和未发布构建保存在 [Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml) 的对应运行中。推送 tag 后，[Release 工作流](../.github/workflows/release.yml) 会在四平台检查通过后将二进制包发布到 [Releases](https://github.com/oonecab/timeline/releases)。
