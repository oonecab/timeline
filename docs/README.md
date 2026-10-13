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

在此添加产品、设计、API、运维、业务规则和用户文档链接。

- [README](../README.md)：构建、CLI/MCP、token 与预算、存储和恢复说明。
- [TIMELINE-SPEC.md](../TIMELINE-SPEC.md)：权威构建规格、§9 测试映射与 §13 验收清单。
- [SPEC-NOTES.md](SPEC-NOTES.md)：规格冲突、实现选择及回归；不暗中改写原规格。
- [ACCEPTANCE.md](ACCEPTANCE.md)：本次 §13 逐项结果、62 项测试分组与实际执行证据。
- [TECH.md](../TECH.md)：技术事实、模块边界、兼容性与技术栈变更索引。
- [environments.md](environments.md)：非秘密变量、路径优先级及运行环境。
- [tests](../tests) 与 [acceptance.py](../scripts/acceptance.py)：可执行验收；运行日志位于 target/acceptance，最终结果由交付统一记录。
