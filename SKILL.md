---
name: timeline
description: 在已接入 tl-mcp 的长任务会话中使用 Timeline，持续记录进展、保留用户约束与承诺、整理摘要、探索分支、归档旧分支，并在每次组装 prompt 前取得上下文。适用于需要跨轮次持续推进或续接的任务。
---

# Timeline 长任务工作流

使用 [TIMELINE-SPEC v1.1.2](TIMELINE-SPEC.md) 定义的工具。预算、去重、水位线与幂等机制由 tl-mcp 负责；Agent 决定何时调用、记录什么。服务接入方式见 [README](README.md#mcp-stdio-接入)，下面的工作流在各平台相同。

## 工作流程

1. **启动与持续记录**
   新任务调用 `timeline_create`；续接任务复用已有 `timeline_id`，用 `timeline_branches` 确认当前分支与 head。用 `timeline_append(role, text)` 记录用户任务、后续消息、关键结果与承诺；新输入先记录，再组装 prompt。保存返回的节点 ID；除 create 外，调用均带 `timeline_id`。

2. **只 pin 三类信息**
   对对应节点调用 `timeline_pin(node)`：① 用户的明确决定或约束；② 长期偏好；③ 未兑现的承诺。临时推测、普通过程记录不 pin；承诺兑现或信息失效后，用 `timeline_unpin(node)` 解除保留。

3. **话题收尾或约 20 轮后整理摘要**
   选择已完成讨论的连续区间，调用 `timeline_summarize(from, to, summary)`。摘要写给“未来的自己”，保留结论、关键依据和未决事项，合计不超过 **5 行**。修正旧摘要时，对同一区间带 `supersede=被替代的最新摘要节点 ID` 创建新版本，保留旧摘要，不原地改写；参考其他摘要时用 `derived_from` 声明其 ID。

4. **用分支探索另一条路线**
   从分歧起点调用 `timeline_fork(node, name)`，再用 `timeline_switch(name)` 切入新分支后继续 append。选定路线后切回对应分支继续工作。不同分支保持独立，不合并分支，也不拼接它们的历史。

5. **停用分支先检查，再 archive**
   完成必要摘要，切换离开待归档分支，再调用 `timeline_archive_check(branch)`；仅在 `eligible=true` 时调用 `timeline_archive_branch(branch)`。否则按 `reasons` 处理后重查，不为归档解除仍有必要的 pin。自 v1.1.1 起，归档会实际搬运分支独有节点，**被摘要覆盖的独有原文也搬运；摘要节点留在热区**，共享节点保持原位。归档不删除原文。

6. **每次组装 prompt 前取上下文**
   调用 `timeline_context_budget(node=当前分支 head, max_tokens=本次上下文预算)`。给模型的会话上下文**只来自返回的 `messages`，按返回顺序原样使用**；不另拼原始历史，不自行截断、筛选或用自制摘要替换。收到 `budget infeasible`，表示强制保留部分及必要摘要已无法容纳于上界：放宽 `max_tokens`，或解除不必要的 pin，再重新调用；成功前不组装替代上下文继续。

**幂等与重试：** MCP 调用从首次请求起带 `idempotency_key`。失败或结果不明时，重试同一操作应复用同一 key 和参数；修正参数或发起新操作时换新 key。短暂故障可有限重试，持续失败则报告阻塞。参数或前置条件错误需要先处理原因，再发起请求。
