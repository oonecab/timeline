# Timeline ADT 设计规格书（TIMELINE-SPEC v1.1.2）

> 日期：2026-10-13 ｜ 状态：构建依据（authoritative）
> v1.1.0 修订：在 v1.0.0 的 6 条公理与 14 个操作契约基础上，吸收 6 项来自论文研读的改进
> （上下文水位线、预算优先级分层、归档三条件、源/派生数据分离、增量 checkpoint 与幂等键、
> 摘要版本链）。新增 2 个操作契约（共 16 个）、2 个 MCP 工具（共 14 个）；DB 格式向后兼容，
> `format_version` 保持 1。变更明细见 §14。
> 本文档是 Timeline 项目的**唯一权威构建规格**。实现者（Claude Code / Codex / 人类）
> 按此文档从零实现，应得到行为一致的系统。
> 早期 Rust 参考实现位于设计者的 `~/workspace/dsaa/timeline/`，当时通过 7 个测试。
> 这是历史来源，不是安装路径；当前仓库的跨平台安装、CI 和验收结果见 [README](README.md) 与 [验收记录](docs/ACCEPTANCE.md)。
> **如本文档与参考实现有冲突，以本文档为准**（文档吸收了实现中发现的精化，
> 例如预算预留、占位复用规则、pin 重复计数的修正）。
> 早期草稿 `~/workspace/dsaa/timeline-adt.md` 同样是历史资料，不再作为构建依据。

---

## 0. 术语与约定

- **ADT**：抽象数据类型。只定义"能做什么"（操作契约），不规定内部表示。
- **节点（Node）**：时间线上的一个点 = 一个时刻的一条消息 + 元数据。
- **分支（Branch）**：git 式命名指针，指向某个节点（该分支的 head）。
- **上下文（Context）**：从根到某节点的路径上的消息序列，可直接喂给 LLM。
- **机制 vs 策略**：ADT 只提供机制（pin / summarize / budget / branch）；
  "何时遗忘、遗忘什么"的策略由调用方（agent prompt / harness）决定，ADT 本体不做。
- 伪代码中的 `u64` = 无符号 64 位整数；`→` 表示返回。
- **水位线（watermark）**：timeline 级别的单调标记（节点 id 或 null）。
  水位之前的历史保证"无裸消息"（每条消息要么被 pin，要么已被摘要覆盖）。
- **归档（archive）**：把满足三条件的分支的独有节点移入冷存储区；
  节点仍可恢复，不是物理删除。
- **幂等键（idempotency_key）**：调用方传入的去重键；服务端对同一 key 的重复调用
  只执行一次（at-least-once 投递 + 服务端去重 = 调用方视角的 exactly-once）。
- **摘要版本（summary_version）**：同一摘要区间被修正时产生新版本，旧版本保留。

---

## 1. 项目概述

**一句话**：给 LLM 对话做 git 式分支管理的持久化数据结构，外加 MCP 服务。

**解决的问题**：
1. 任意历史节点可 fork 出"平行宇宙"，形成分支树，随时切换；
2. token 预算压力下组装上下文：重要节点钉住（pin），中间区间自动降级为摘要；
3. 遗忘可逆：摘要是新节点，原文永远保留，可审计、可回滚。

**设计哲学**：
- 节点（近似）不可变 → fork 零拷贝，与 git commit 同构；
- 取舍作用在"视图"上，不销毁树；
- 刻意极简：想清楚"不需要什么"（无 merge、无自动策略）和"需要什么"同样重要。

---

## 2. 分层架构

```
┌──────────────────────────────────────────────┐
│ Layer 3: MCP Server (tl-mcp)                 │  stdio JSON-RPC；14 个工具；无状态转调
├──────────────────────────────────────────────┤
│ Layer 2: CLI (tl)                            │  人类 / 脚本入口；JSON 输出
├──────────────────────────────────────────────┤
│ Layer 1: Storage                             │  单 JSON 文件持久化；原子写；文件锁串行化
├──────────────────────────────────────────────┤
│ Layer 0: Core ADT                            │  纯数据结构：无 I/O、无时钟、无随机数
│                                              │  （ID 计数器由 Storage 层注入）
└──────────────────────────────────────────────┘
```

**依赖规则**：上层依赖下层，下层不知道上层的存在。
Core ADT 必须是可单元测试的纯逻辑——第 9 节的验收测试全部针对 Layer 0（+ Storage  round-trip）。

**推荐实现语言**：Rust（与参考实现一致：内存安全，可按平台分发独立可执行文件；CLI 与 MCP 服务各一个）。
规格本身语言无关；用其他语言实现必须逐条满足操作契约与公理。

**推荐目录布局**：

```
timeline/
├── src/
│   ├── lib.rs        # Layer 0: 纯 ADT 核心
│   ├── storage.rs    # Layer 1: 持久化
│   ├── bin/tl.rs     # Layer 2: CLI
│   └── bin/tl-mcp.rs # Layer 3: MCP server
├── tests/
│   └── axioms.rs     # 第 9 节：六条公理的验收测试
└── README.md         # 构建、测试、接入说明
```

---

## 3. 数据模型

### 3.1 Message

| 字段   | 类型   | 说明 |
|--------|--------|------|
| role   | string | `user` / `assistant` / `system` / `summary`（允许扩展，但这四个为标准） |
| text   | string | 内容 |
| tokens | u64    | 调用方提供的 token 数。**ADT 不做分词**，保持纯粹与可测试 |

缺省 token 估计器（调用方未提供时）：`E(text) = max(1, ceil(字符数 / 4))`，
字符按 Unicode 标量计数。实现必须在文档中声明此公式（保证测试可复现）。

### 3.2 TLNode（节点）

| 字段          | 类型                        | 说明 |
|---------------|-----------------------------|------|
| id            | string                      | DB 全局唯一，格式 `n{k}`，k 为 DB 级单调计数器 |
| parent        | string \| null              | 根节点为 null |
| children      | string[]                    | 子节点 id 列表 |
| message       | Message \| null             | 根节点为 null |
| pinned        | bool                        | 是否被钉住，默认 false |
| prefix_tokens | u64                         | 根→本节点的累计 tokens（含本节点）；根为 0 |
| summary_range | `{from,to}` \| null         | 仅摘要节点有；`from`/`to` 为节点 id |
| summary_version | u64                      | 摘要节点：版本号，默认 1；每次修正递增 |
| supersedes | string \| null              | 摘要节点：被本版本替代的旧版本节点 id；首版为 null |
| derived_from | string[]                  | 摘要节点：本摘要撰写时参考的摘要节点 id 列表；默认空 |
| stale | bool                          | 摘要节点：**派生字段**，默认 false；当 `derived_from` 中的任一摘要发布了新版本时置 true |

### 3.3 Timeline

| 字段           | 类型              | 说明 |
|----------------|-------------------|------|
| id             | string            | 格式 `tl{k}`，DB 级单调计数器 |
| nodes          | map id → TLNode   | |
| branches       | map name → head   | 初始 `{"main": root}` |
| current_branch | string            | 必在 `branches` 中 |
| watermark      | string \| null    | 水位线（v1.1 新增）；null 表示未建立；语义见 §3.4 不变量 7/8 |
| archive        | map id → TLNode   | 归档区（v1.1 新增）；被归档的节点，仍可恢复 |

### 3.4 不变量（实现必须时刻维护，测试必须覆盖）

1. 每个非根节点有且仅有一个 parent；parent 的 `children` 包含它。
2. `prefix_tokens(n) = prefix_tokens(parent(n)) + tokens(message(n))`；根为 0。
3. `branches` 的每个 head 都存在于 `nodes`；`current_branch` 必在 `branches` 中。
4. `summary_range.from` 必须是 `summary_range.to` 的祖先（可相等），且二者在同一分支上。
5. **节点近似不可变**：除 `pinned` 标志和 `children` 列表（追加子节点时）外，
   任何操作不得修改已有节点的字段。`message` / `parent` / `prefix_tokens` /
   `summary_range` 一经创建永不改变。
6. `tokens(node)` 必须为 O(1)（直接读 `prefix_tokens`，不得遍历重算）。
7. **水位完整性**：若 `watermark = W ≠ null`，则 root→W 路径上每个非根节点 n 满足：
   `n.pinned ∨ (∃摘要节点 s: n ∈ [s.from, s.to])`（占位摘要也算覆盖）。
   即水位之前无"裸"消息。
8. **水位单调**：任何操作不得使 watermark 回退（新值不得是旧值的祖先）。
   watermark 只在 `summarize` 成功后自动推进：若 root→to 路径满足不变量 7 的条件，
   且（watermark 为 null，或 watermark 是 to 的祖先），则 watermark 置为 to。
9. **引用计数语义**：`refcount(n) = 经过 n 的分支 head 数 + 覆盖 n 的摘要数 + (pinned?1:0)`。
   refcount 归零的节点才允许物理删除；归档操作只搬运分支独有节点
   （分支引用计数为 1 且无 pin）。被摘要覆盖的节点允许搬运：
   其语义已由摘要节点承载（摘要为独立节点，不随归档移动），
   且原文节点永不删除（§5.3 保证 3），归档仅为冷热物理位置搬运，仍可恢复。
   refcount 为派生数据，启动时重建。
10. **摘要版本链**：`supersedes` 形成单链；旧版本永保留（可逆）；`stale` 为派生标记，
    由 `derived_from` 关系重算得出，不得持久化脏值（见 §6.3）。

### 3.5 ID 生成

DB 级维护两个单调计数器 `next_timeline`、`next_node`。
`create` 取用 `tl{next_timeline}` 并递增；`append`/`summarize` 取用 `n{next_node}` 并递增。
决定性、可测试、无随机。

---

## 4. 操作契约（16 个）

约定：所有操作返回显式错误（Result / 异常），**不得静默失败**。
**幂等约定（v1.1 新增）**：变更操作必须对重复调用安全——同参数重复调用结果相同，
不产生重复副作用（`pin` 已 pin 的节点、`summarize` 同区间同文本等）。
MCP 层另有 `idempotency_key` 服务端去重（见 §7.1）。
常见错误：`节点不存在`、`分支名已存在`、`分支不存在`、`不能删除当前分支`、
`摘要区间不连续`（from 不是 to 的祖先）。

| # | 操作 | 签名 | 前置 / 后置 / 复杂度 |
|---|------|------|----------------------|
| 1 | create | `create() → timeline_id` | 新时间线：根节点（message=null），`branches={"main": root}`。O(1) |
| 2 | append | `append(role, text, tokens?) → node_id` | 在 current_branch 的 head 后追加；新节点 parent=head；head 更新为新节点；`prefix_tokens` 按不变量 2 计算。O(1) |
| 3 | fork | `fork(node_id, name) → void` | name 在本 timeline 内唯一（重复则错）；新分支 head=node_id；**零拷贝**：不复制任何节点。O(1) |
| 4 | switch | `switch(name) → void` | name 必须存在；设置 current_branch。O(1) |
| 5 | drop_branch | `drop_branch(name) → void` | name 必须存在；**不能删除当前分支**（错）；`main` 与其他分支规则相同。只删分支指针，不删节点。O(1) |
| 6 | context | `context(node_id) → Message[]` | 返回 root→node 路径上的消息（跳过根的 null），从旧到新。O(路径长) |
| 7 | tokens | `tokens(node_id) → u64` | 返回 `prefix_tokens`。**O(1)**（不变量 6） |
| 8 | pin | `pin(node_id) → void` | 置 `pinned=true`。O(1) |
| 9 | unpin | `unpin(node_id) → void` | 置 `pinned=false`。O(1) |
| 10 | summarize | `summarize(from_id, to_id, summary_text, tokens?, derived_from?, supersede?) → node_id` | from 须为 to 的祖先（可相等），否则错；新节点为 `parent(from)` 的子节点，`message.role="summary"`，`summary_range={from,to}`，`summary_version=1`；**原文节点不动**（遗忘可逆）。`supersede` 给出时：须为同区间的已回填摘要，新节点 `summary_version=旧版+1`、`supersedes=旧版 id`，旧版保留；所有 `derived_from` 含旧版的摘要置 `stale=true`（级联）。O(1)+O(区间长度校验) |
| 11 | context_budget | `context_budget(node_id, max_tokens) → BudgetResult` | 精确算法见 §5。`BudgetResult={messages, total_tokens, summaries_created}` |
| 12 | branches | `branches() → [{name, head, current}]` | O(分支数) |
| 13 | node | `node(node_id) → TLNode` | 返回节点详情（含 children/pinned/prefix_tokens/summary_range）。O(1) |
| 14 | （隐式） | ID 计数器递增 | 由 Storage 层在每次 create/append/summarize（含占位）时执行 |

### 4.1 归档与水位相关操作（v1.1 新增，操作 15–16）

| # | 操作 | 签名 | 前置 / 后置 / 复杂度 |
|---|------|------|----------------------|
| 15 | archive_check | `archive_check(branch) → {eligible: bool, reasons: string[]}` | 纯查询。按**归档三条件**逐项判定分支是否可归档：① **水位条件**：`watermark ≠ null`，且 head 为 watermark 本身或其祖先（分支内容已全部越过水位线）；② **无 pin**：root→head 路径上无 `pinned` 节点；③ **无未回填的摘要依赖**：不存在仍为 PLACEHOLDER 模板的摘要节点，其 `summary_range` 与该路径相交。`reasons` 列出不满足的条件。O(路径长 + 摘要数) |
| 16 | archive_branch | `archive_branch(branch) → {archived: string[]}` | 前置：`archive_check(branch).eligible` 必须为 true，否则错；不能归档当前分支。语义：将该分支**独有**节点（经过该节点的分支 head 数 == 1、且无 pin）移入 `archive` 区；被摘要覆盖的独有节点允许搬运（其语义已由摘要节点承载，摘要为独立节点不随归档移动）；共享前缀节点与其他分支共享的节点不动；分支指针删除。归档节点仍可恢复（恢复操作预留给 v1.2）。每次归档写入审计日志。后置：`archive_check` 三条件保持不变；独有 pin 节点保留在热区（pin 语义优先）。O(路径长) |

---

## 5. `context_budget` 精确算法

这是本 ADT 最微妙的部分。实现必须满足 §5.3 的保证；§5.2 伪码为常规路径的参考实现，
保证优先于伪码字面步骤。（原"实现必须逐行遵循"在此条下无法与保证同时成立，
特此澄清，见 §5.5。）

### 5.1 常量

- **RESERVE**：自动摘要占位模板文本的 token 数上确界。实现常量，**必须在文档中声明**。
  参考实现取 **32**。
- **PLACEHOLDER 模板**（占位文本，实现可翻译但必须等长量级并重算 RESERVE）：
  `[自动摘要占位：{count} 条消息（{from}→{to}），调用 summarize 写入真实摘要]`

### 5.2 伪代码

```
context_budget(node_id, max_tokens) → {messages, total_tokens, summaries_created}:
    path = 从 root 到 node_id 的节点 id 列表   # path[0]=root, path[n-1]=node_id
    n = len(path)
    kept = [False] * n
    kept[0] = True                  # 根无消息：恒保留，不占预算
    kept[n-1] = True                # Tier 0 目标节点：恒保留（问的就是它的上下文）
    spent = tokens(path[n-1])
    # Tier 1 pin 优先：被 pin 的节点永不被省略
    for id in path:
        if node(id).pinned and not kept[index(id)]:
            kept[index(id)] = True
            spent += tokens(id)
    # Tier 2 语义枢纽优先（v1.1 新增）：被摘要覆盖的节点是语义枢纽，
    # 优先于纯时间贪心保留。covers(n) = 覆盖 n 的摘要节点数（派生索引，O(1) 查）
    for id in reversed(path):       # 从新到旧
        i = index(id)
        if not kept[i] and covers(id) > 0 and spent + tokens(id) <= max_tokens:
            kept[i] = True
            spent += tokens(id)

    in_drop = False
    for i from n-2 downto 1:        # 从新到旧贪心
        if kept[i]:
            in_drop = False
            continue
        t = tokens(path[i])
        if in_drop:
            if spent + t <= max_tokens:
                kept[i] = True; spent += t; in_drop = False
            # 否则：丢弃区间延长，不额外占预算
        elif spent + t <= max_tokens:
            kept[i] = True; spent += t
        else:
            spent += RESERVE         # 开启新摘要区间：为占位节点预留成本
            in_drop = True

    # 组装输出
    out = []; summaries_created = []
    i = 1
    while i <= n-1:
        if kept[i]:
            out.push(message(path[i])); i += 1
        else:
            a = i
            while i <= n-1 and not kept[i]: i += 1
            b = i - 1                # 丢弃区间 [a..b]，1<=a<=b<=n-2
            s = get_or_create_placeholder(parent=path[a-1], from=path[a], to=path[b])
            summaries_created.push(s.id)   # 复用时也返回 id，但不重复创建
            out.push(s.message)
    total = sum(tokens(m) for m in out)
    return {messages: out, total_tokens: total, summaries_created}
```

`get_or_create_placeholder`（占位复用规则，防 DB 膨胀）：
- 若 `parent` 的子节点中已存在 `role=summary`、`summary_range={from,to}`、
  且 text 仍为 PLACEHOLDER 模板（未被回填）的节点 → 复用它，不新建。
- 否则新建占位节点：`message={role:"summary", text:模板, tokens:RESERVE}`，
  `summary_range={from,to}`，parent 为区间父节点。**占位节点的 tokens 恒为 RESERVE**。

### 5.3 保证（必须成立，测试必须断言）

1. `total_tokens ≤ max_tokens + RESERVE` ——**除非**被 pin 的节点本身已超预算，
   此时 pin 优先（公理 5 的例外）。
2. 目标节点恒在输出中。
3. 每个被丢弃的极大连续区间恰对应一个摘要消息；**原文节点全部保留**（遗忘可逆）。
4. 时间复杂度 O(路径长)；除占位节点创建外不分配大内存。
5. **去重**：`total_tokens` 按去重后的消息集合计算；单次调用内路径天然无重复节点；
   占位节点的 tokens 恒为 RESERVE，不重复计（为未来多路径扩展预留的不变量）。

### 5.4 水位优化（v1.1 新增，推荐实现）

若 `watermark` 为 `node_id` 的祖先，实现**可以**从 watermark 之后开始扫描：
watermark 之前的区间不再创建新占位，直接采用已存在的摘要节点（不变量 7 保证
每个区间已有摘要或 pin）。此优化不得改变 §5.3 的任何保证；测试必须断言
优化开/关下输出一致（§9）。

> 与参考实现的差异（以本规格为准）：参考实现在 pin 循环中可能对已保留的目标节点
> 重复计 tokens（偏保守）；本规格要求 `not kept` 检查。参考实现每次调用都新建占位；
> 本规格要求复用规则。

### 5.5 预算不可行（v1.1.2 新增）

定义输入 `(path, max_tokens)` 为**预算可行**，当且仅当存在满足 §5.3 保证 1–3 的
kept 选择。`context_budget` 的契约修订为：

1. **输入可行时**：必须返回满足 §5.3 全部保证的结果。§5.2 伪码为常规路径参考实现；
   允许等价的修复循环（合并丢弃区间、复用占位等），只要保证成立。
2. **输入不可行时**（如强制保留部分 Tier 0+Tier 1 已超上界，或任何合法 kept 选择
   都迫使输出超界）：必须返回显式错误 `budget infeasible`，
   错误载荷含 `{required_tokens, max_tokens}`，**不得返回超预算的静默结果**。

反例（字面伪码违反自身保证的输入，§9 测试必须断言）：
链 tokens `[root=0, 1000, 1(pin), 1000, target=0]`、`max_tokens=1` 时，
字面贪心产生两个丢弃区间（被 pin 节点隔开），各占位 32，
输出 `32+1+32=65 > max+32=33`，违反 §5.3 保证 1。
此输入为预算不可行，必须返回 `budget infeasible` 错误。

---

## 6. 存储层（Storage）

### 6.1 文件与格式

- 单 JSON 文件。默认路径 `~/.timeline.json`，可被环境变量 `TIMELINE_DB` 或
  `--db PATH` 覆盖（优先级：`--db` > `TIMELINE_DB` > 默认）。
- 顶层 schema（`format_version=1`）：

```json
{
  "format_version": 1,
  "next_timeline": 1,
  "next_node": 7,
  "timelines": {
    "tl0": {
      "id": "tl0",
      "nodes": {
        "n0": {"id":"n0","parent":null,"children":["n1"],"message":null,
               "pinned":false,"prefix_tokens":0,"summary_range":null},
        "n1": {"id":"n1","parent":"n0","children":["n2","n3"],
               "message":{"role":"user","text":"引理2看不懂","tokens":20},
               "pinned":true,"prefix_tokens":20,"summary_range":null},
        "n2": {"id":"n2","parent":"n1","children":[],
               "message":{"role":"assistant","text":"我们一步步来","tokens":30},
               "pinned":false,"prefix_tokens":50,"summary_range":null}
      },
      "branches": {"main":"n2"},
      "current_branch": "main"
    }
  }
}
```

**v1.1 新增字段**（向后兼容，缺失时按缺省值解释；`format_version` 保持 1）：
timeline 级：`watermark`（默认 null）、`archive`（默认空 map）；
node 级：`summary_version`（默认 1）、`supersedes`（默认 null）、
`derived_from`（默认空数组）、`stale`（默认 false，派生）。

### 6.2 写入语义

- **原子写**：写临时文件 → fsync → rename 覆盖。任何时刻 DB 文件都是完整快照。
- **串行化**：所有写操作必须持有文件独占锁（`flock`）；读操作持共享锁。
  MCP server 内再串行化请求（单线程处理 tools/call）。
- **备份**：每次成功写入前，将上一版本复制为 `<db>.bak`（推荐，不强制）。
- **文件权限**：新建 DB 文件权限 `0600`（对话内容是隐私）。
- **损坏恢复**：启动时校验 `format_version`；版本不匹配或 JSON 损坏 → 报错并指引用 `.bak` 恢复，
  不得静默覆盖。
- **增量 checkpoint（v1.1 新增，推荐）**：除全量原子写快照外，实现**可以**维护 append-only
  的增量操作日志（自上次全量 checkpoint 后的操作序列）；启动时先载入最近快照再 replay 日志。
  增量日志本身也要求原子追加；何时做全量 checkpoint 由实现决定（建议按操作数或时间阈值），
  文档化。
- **幂等键去重（v1.1 新增）**：MCP/CLI 的变更操作接受可选 `idempotency_key`；
  服务端维护 `key → 首次结果` 映射（7 天过期，实现可配）；同一 timeline + key 的重复调用
  直接返回首次结果，不重复执行、不新增节点。映射写入审计日志备查。

---

### 6.3 源数据与派生数据（v1.1 新增）

**规则**：只有"丢了重建不出来"的数据才必须持久化；其余派生数据启动时重建，
实现必须在文档中列出派生字段清单。

| 类别 | 字段 | 说明 |
|------|------|------|
| 源数据（必须落盘） | nodes 的 message/parent/children/pinned/summary_range/summary_version/supersedes/derived_from；branches；watermark；archive；next_* 计数器；审计日志 | 丢失不可重建 |
| 派生数据（可重建，落盘可选） | prefix_tokens；covers 索引（节点→覆盖它的摘要列表）；refcount；占位复用索引；stale 标记 | 启动时由源数据重建；损坏时可丢弃重建，不得因此报错 |

损坏恢复时派生数据不一致 → 静默重建，不得视为 DB 损坏。

---

## 7. MCP Server 层（tl-mcp）

### 7.1 协议

- 传输：stdio 上的 JSON-RPC 2.0。兼容 MCP `2024-11-05` 及以后。
- 握手：`initialize` → 返回 `serverInfo: {name:"timeline", version:"1.1.0"}`；
  `notifications/initialized`；`tools/list` 返回 14 个工具；`tools/call` 执行；
  `ping` 保活。
- **幂等键**：所有 `tools/call` 接受可选 `idempotency_key: string`；
  语义见 §6.2（同一 timeline + key 去重 7 天）。
- 错误：JSON-RPC error 对象，`code=-32000`，`message` 为人类可读的原因
  （只给原因，不泄露内部路径/堆栈）。
- 无状态：每个 `tools/call` = 读 DB → 执行 Layer 0 操作 → 写 DB → 返回。
- 启动参数：`tl-mcp --db PATH`（缺省走 §6.1 的路径规则）。

### 7.2 十四个工具

通用：所有工具的输入都含 `timeline_id: string`（下表省略）。
输出均为 JSON。`tokens` 参数缺省时用 §3.1 的估计器。

| 工具名 | 输入（除 timeline_id） | 输出 |
|--------|----------------------|------|
| `timeline_create` | — | `{"timeline_id":"tl0"}` |
| `timeline_append` | `role`（枚举）, `text`, `tokens?` | `{"node":"n12"}` |
| `timeline_fork` | `node`, `name` | `{"branch":"追问引理2","head":"n5"}` |
| `timeline_switch` | `name` | `{"current_branch":"追问引理2"}` |
| `timeline_drop_branch` | `name` | `{"dropped":"追问引理2"}` |
| `timeline_context` | `node` | `[{"role":"user","text":"…","tokens":20}, …]`（旧→新） |
| `timeline_context_budget` | `node`, `max_tokens`（整数 ≥0） | `{"messages":[…],"total_tokens":87,"summaries_created":["n20"]}` |
| `timeline_pin` | `node` | `{"pinned":"n1"}` |
| `timeline_unpin` | `node` | `{"unpinned":"n1"}` |
| `timeline_summarize` | `from`, `to`, `summary`, `tokens?`, `derived_from?`（string[]）, `supersede?`（被替代的旧版本节点 id） | `{"node":"n21"}`。`supersede` 给出时创建新版本（旧版本保留，`supersedes` 指向上版）；`derived_from` 声明参考的摘要，用于级联 stale 标记 |
| `timeline_branches` | — | `[{"name":"main","head":"n2","current":true}, …]` |
| `timeline_node` | `node` | `{id,parent,children,role,text,tokens,pinned,prefix_tokens,summary_range}`（根的 role/text 为 null） |
| `timeline_archive_check` | `branch` | `{"eligible":true,"reasons":[]}`（v1.1 新增；语义见 §4.1） |
| `timeline_archive_branch` | `branch` | `{"archived":["n7","n8"]}`（v1.1 新增；语义见 §4.1） |

示例 inputSchema（`timeline_context_budget`，其余类推）：

```json
{
  "type": "object",
  "properties": {
    "timeline_id": {"type": "string"},
    "node": {"type": "string"},
    "max_tokens": {"type": "integer", "minimum": 0}
  },
  "required": ["timeline_id", "node", "max_tokens"]
}
```

---

## 8. CLI 层（tl）

与 MCP 工具一一对应，供人类与脚本使用。

```
tl [--db PATH] [--timeline TL] <command> [args]

tl create
tl append --role user --text "引理2看不懂" [--tokens 20]
tl fork --node n2 --name 追问引理2
tl switch --name 追问引理2
tl drop-branch --name 追问引理2
tl context --node n4
tl budget --node n4 --max 100000
tl pin --node n1
tl unpin --node n1
tl summarize --from n1 --to n3 --summary "前三轮在讨论背景" [--tokens 40] [--derived-from n9,n10] [--supersede n7]
tl branches
tl node --node n1
tl archive-check --name 旧分支
tl archive-branch --name 旧分支
```

- `--timeline` 缺省为 DB 中最早创建的 timeline；MCP 则每次显式传 `timeline_id`。
- 输出：JSON 到 stdout（与 §7.2 的输出形状一致）。
- 错误：`{"error":"原因"}` 到 stderr，exit code 非零；成功 exit 0。
- `--db` / `TIMELINE_DB` / 默认路径规则同 §6.1。

---

## 9. 六条公理 → 验收测试

每个公理至少对应一个测试（`tests/axioms.rs`）。实现者必须全部通过；
验收方（用户）对照此表验收。

| 公理 | 测试名 | Given / When / Then |
|------|--------|---------------------|
| A1 不可变 | `axiom_immutability` | Given 一条链；When 在其后 append/fork/pin；Then 老节点的 message/parent/prefix_tokens 与操作前逐字节相同 |
| A1 前缀和 | （同上） | Then 任一节点的 `tokens()` == 其路径 tokens 之和，且为 O(1)（读字段） |
| A2 零拷贝 fork | `axiom_fork_zero_copy` | Given 链 root→n2；When `fork(n1,"b")`；Then 新分支 head==n1 且**节点总数不变**；再在 b 上 append，Then main 的 context 不变 |
| A3 路径即上下文 | `axiom_path_is_context` | Given 链 + 分支；When `context(n)`；Then 返回恰为 root→n 路径消息序列（旧→新，跳过根） |
| A4 分支隔离 | `axiom_branch_isolation` | Given fork 出 b；When 在 b 上 append 三条；Then main 的 head/context/tokens 全不变 |
| A5 预算保证 | `axiom_budget_bound` | Given 10 节点链（每节 50t）；When `context_budget(n10, 150)`；Then `total_tokens ≤ 150+RESERVE` 且含 n10 的消息；**property**：随机链长/随机 tokens/随机预算 fuzz 1000 次，上界恒成立；**不可行输入**：链 tokens `[root=0, 1000, 1(pin), 1000, target=0]`、`max_tokens=1` 时，Then 返回 `budget infeasible` 错误（载荷含 `required_tokens`/`max_tokens`），不得返回超预算结果 |
| A5 pin 优先 | `axiom_pin_beats_budget` | Given 同上；When `pin(n2)` 后 `context_budget(n10, 150)`；Then n2 的消息必在输出中（即使 total 超预算——pin 优先） |
| A6 可逆遗忘 | `axiom_summarize_reversible` | Given 链；When `summarize(n2,n5,"…")`；Then n2..n5 仍可 `context` 取回原文；摘要节点 `summary_range={n2,n5}` 且为 parent(n2) 的子节点 |
| A6 占位复用 | `axiom_placeholder_dedup` | When 对同一区间调两次 `context_budget`；Then 第二次不新增占位节点（`summaries_created` 返回同一 id） |
| A5 优先级分层 | `axiom_priority_tiers` | Given 长链 + 对旧区间做过摘要；When 预算紧张到只能二选一；Then 被摘要覆盖的旧节点优先于未被覆盖的旧节点保留（Tier 2 优先于 Tier 3 贪心） |
| 不变量 7 水位完整性 | `axiom_watermark_integrity` | When `summarize` 覆盖 root→to 全路径后；Then `watermark==to` 且 root→to 无裸节点 |
| 不变量 8 水位单调 | `axiom_watermark_monotonic` | fuzz 随机操作序列；Then watermark 永不回退 |
| 归档三条件 | `axiom_archive_three_conditions` | Given 三种分支（全满足 / 缺水位 / 有 pin / 有未回填占位）；When `archive_check`；Then eligible 与 reasons 逐项正确；`archive_branch` 只搬运独有节点，共享前缀不动 |
| 幂等键 | `axiom_idempotency_key` | When 同一 `idempotency_key` 调两次 `append`/`summarize`；Then 节点总数不变，两次返回同一 id |
| 版本链 | `axiom_summary_version_chain` | When 用 `supersede` 修正摘要；Then 新版本 `summary_version` 递增、`supersedes` 指旧版、旧版仍可取回；`derived_from` 引用旧版的摘要被标 `stale` |

**冒烟测试**（CLI + MCP，脚本化，参考实现已跑通）：
`initialize → tools/list（14 个）→ create → append×2 → fork → switch → append → branches → context → context_budget → pin → node → 错误用例（fork 不存在的节点应返回 JSON-RPC error）`。

---

## 10. 安全与健壮性

1. **输入校验**：node id 必须匹配 `^n[0-9]+$`；分支名非空（允许 Unicode，禁 `/` 与空字符串）；
   `max_tokens` 为非负整数；`role` 为已知枚举（未知 role 报错，不静默接受）。
2. **摘要审计（必做）**：所有 `summarize` 与占位创建必须写入**只增审计日志**
   （DB 内 `audit: [{ts, op, from, to, node}]` 数组或独立文件二选一，实现文档化）。
   原文节点永不删除 → 任何摘要都可回滚到原文。这是公理 6 的操作化，
   也是对"可写记忆是 prompt 注入驻留点"的直接防御（参考《Context Language Models》第 6 节）。
3. **最小权限**：DB 文件 `0600`；MCP server 只读写 DB 文件，不访问网络；
   错误信息不含内部路径。
4. **资源上限**：单条 message text 长度上限（建议 1MB，超限报错）；
   `context`/`context_budget` 返回的消息数上限（建议 10 万条，超限报错并提示用 budget）。

---

## 11. 刻意不做（Non-goals）

1. **无 merge**：两个对话分支在语义上无法像代码那样合并，硬加只会污染设计。
2. **无自动策略**：ADT 不决定"何时摘要、摘要什么"；策略层在外（agent prompt / harness）。
3. **无多写者并发**：单文件 + 文件锁串行化；不需要分布式一致性。
4. **无加密 / 网络同步**：以后再说；当前假设单机可信环境。
5. **无向量检索 / 语义搜索**：那是策略层或外围工具的事，不是 ADT 的职责。

## 12. 决策日志（为什么这样设计）

| 决策 | 理由 |
|------|------|
| 分支模型（git branch）而非纯提交链 | 对话需要"当前宇宙"概念 → head 指针是刚需 |
| 节点近似不可变（仅 pinned/children 可变） | fork 零拷贝；历史可审计；与函数式持久化数据结构同构 |
| tokens 由调用方提供，ADT 不分词 | 保持 ADT 纯粹、可测试；分词是环境相关的 |
| 预算算法 = 贪心 + RESERVE 预留 | O(n) 简单、可证明上界；预留解决"占位自身占 token"的自指问题 |
| 占位摘要复用规则 | 防 DB 膨胀；调用方 `summarize` 回填时复用同一节点 |
| pin 优先于预算 | 重要记忆的语义优先于数字预算；上界保证为此让路并显式声明 |
| 摘要只增、原文永删不掉 | 可逆遗忘；对抗摘要污染（prompt 注入驻留）的审计/回滚基础 |
| MCP 而非各家原生插件 | ADT 与 harness 无关；Claude Code / Codex / Cursor / dsh 都能用；dsh 尚处 preview，API 不稳定 |
| Rust 实现 | 内存安全，可按平台分发 CLI 与 MCP 两个独立可执行文件；规格本身语言无关 |
| 上下文水位线（v1.1） | Dataflow 的 watermark 思想：单调的"完整性度量"；预算扫描只走水位之后；由 summarize 自动推进，不新增工具 |
| 预算优先级分层（v1.1） | QwenGyre 的 role priority：被摘要覆盖的节点是语义枢纽，优先于纯时间贪心；去重按集合计（防未来多路径扩展） |
| 归档三条件（v1.1） | IBM Streams Beam runner 的 keyed GC：水位 / 无 pin / 无未回填摘要依赖；引用计数归零才可动；归档≠删除 |
| 源/派生分离（v1.1） | HDFS"可重建的状态不值得持久化"：派生索引启动重建，写放大只付给源数据 |
| 增量 checkpoint + 幂等键（v1.1） | MillWheel 的 per-key checkpoint / exactly-once via 幂等：append-only 增量日志；MCP at-least-once 投递 + 服务端去重 |
| 摘要版本链（v1.1） | Dataflow 的 retraction：修正 = 新版本 + 旧版保留；`derived_from` 下游级联标 stale |

## 13. 交付验收清单（给验收方逐项打勾）

- [ ] 单元测试全过：§9 的 15 个测试（含 fuzz property test）无失败
- [ ] 不变量检查：随机操作序列后断言 §3.4 的 10 条不变量（含 prefix_tokens 一致性）
- [ ] CLI 全命令冒烟通过（§9 脚本），JSON 输出形状与 §7.2 一致
- [ ] MCP 全流程通过：initialize → tools/list（14 个）→ tools/call 全工具 → 错误用例返回 JSON-RPC error
- [ ] 预算上界：fuzz 1000 次随机场景，`total ≤ max_tokens + RESERVE` 恒成立（pin 超预算的用例除外，见 A5）
- [ ] 审计日志：每次 summarize/占位创建都有记录；可凭记录回滚到原文
- [ ] 文档：在 README 中声明 RESERVE 取值、token 估计器公式、审计日志位置
- [ ] 水位线：fuzz 后不变量 7/8 恒成立；§5.4 优化开/关下 `context_budget` 输出一致
- [ ] 归档三条件：`archive_check` 逐项正确；`archive_branch` 不搬运共享节点
- [ ] 幂等键：同一 key 重复调用不产生重复节点，返回同一 id
- [ ] 摘要版本链：修正产生新版本且旧版可回滚；下游 `stale` 标记正确
- [ ] 文档：在 README 中声明派生字段清单、水位语义、增量 checkpoint 策略
- [ ] `tl-mcp --db` 与 `TIMELINE_DB` 覆盖规则实测正确

## 14. 版本与变更

- TIMELINE-SPEC v1.1.2（2026-10-13）：澄清预算契约（Codex 实现 review 阻塞项 2，
  Corentin 确认）——新增 §5.5「预算不可行」：保证优先于伪码字面步骤；
  不可行输入必须返回 `budget infeasible` 错误（载荷 `{required_tokens, max_tokens}`），
  不得返回超预算的静默结果；§9 `axiom_budget_bound` 追加不可行输入断言。
  DB 格式与 MCP 工具集无变化（`format_version=1`，工具集版本 1.1.0）。

- TIMELINE-SPEC v1.1.1（2026-10-13）：修订归档语义（Codex 实现 review 阻塞项 1，
  Corentin 裁决选项 a）——op 16 放宽搬运条件，允许搬运被摘要覆盖的分支独有节点
  （分支引用计数为 1 且无 pin；`archive_check` 三条件不变）；§3.4 不变量 9 同步修订；
  修正 §13"6 条不变量"笔误为 10 条。DB 格式与 MCP 工具集无变化
 （`format_version=1`，工具集版本 1.1.0）。

- TIMELINE-SPEC v1.1.0（2026-10-13）：吸收论文研读的 6 项改进——
  ① 上下文水位线（§3.3/§3.4 不变量 7/8，§5.4）；
  ② 预算优先级分层 Tier 2 + 去重声明（§5.2/§5.3）；
  ③ 归档三条件与引用计数（§3.4 不变量 9，§4.1 操作 15–16）；
  ④ 源/派生数据分离（§6.3）；
  ⑤ 增量 checkpoint 与幂等键（§6.2，§7.1）；
  ⑥ 摘要版本链与级联 stale（§3.2/§3.4 不变量 10，操作 10 扩展）。
  新增 2 个操作契约、2 个 MCP 工具；DB 新增字段全部向后兼容。
- TIMELINE-SPEC v1.0.0（2026-10-06）：首个完整构建规格（原名 SPEC.md）。
- `format_version=1`：DB 文件格式版本；v1.1 新增字段均为可选（缺失按缺省值解释），
  不兼容变更时递增并写迁移说明。
- MCP 工具集版本 `1.1.0`：新增工具必须向后兼容（只增不改）；破坏性变更升级大版本。

---

*附：把本文件直接交给 Claude Code / Codex，指令可以是：*
*"按 TIMELINE-SPEC.md 从零实现 Timeline 项目（Rust），完成后跑 §13 验收清单，把结果逐项贴出来。"*
