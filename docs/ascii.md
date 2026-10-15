# ASCII 终端图

`tl export --format ascii` 将所选时间线渲染为纯文本分支树，可以直接在 Bash、PowerShell 等终端查看，无需安装绘图工具或启动服务。ASCII 限定的是连线与标记；中文、emoji 等原文仍以 UTF-8 保存和输出。此页是终端图展示与输出行为的权威说明。

## 命令

下面假设 `tl` 已加入 PATH；从项目的本机构建运行时，Windows 使用 `.\target\release\tl.exe`，Bash 使用 `./target/release/tl`，Git Bash 调用 Windows 产物使用 `./target/release/tl.exe`。

```sh
# 直接查看；stdout 只包含文本图，不附加 JSON
tl --db conversation.timeline.json --timeline tl0 export --format ascii

# 保存文件；文件内为 UTF-8 文本图，stdout 为 JSON 结果
tl --db conversation.timeline.json --timeline tl0 export --format ascii --output task.txt
```

`--db` / `TIMELINE_DB` / 默认路径和时间线选择规则沿用现有 CLI。文件输出的父目录必须存在，已有文件拒绝覆盖；`export` 继续拒绝 `--idempotency-key`。`drawio` 仍为默认格式，必须传 `--output`，原有用法不变。

Bash 分页与重定向示例：

```bash
tl --db conversation.timeline.json export --format ascii | less
tl --db conversation.timeline.json export --format ascii > task.txt
```

PowerShell 查看前几行、重读已保存文件：

```powershell
tl --db conversation.timeline.json export --format ascii | Select-Object -First 20
Get-Content -Encoding UTF8 task.txt
```

如果旧版 PowerShell 的外部程序管道解码出现乱码，可在当前会话设置 `[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)`。需要编码稳定的文件时使用 `--output`，避免 shell 对重定向内容重新编码。shell 的 `>` 是否覆盖已有文件由 shell 决定；只有 `--output` 使用 Timeline 的拒绝覆盖保护。分页器或下游命令提前关闭管道时正常退出，不打印 Rust panic。

## 布局与标记

```text
Timeline tl0 | current="main"
History: parent links | HEAD*: current branch | COLD: archived
Summary links are annotations, not merges.

[n0 root]
|
[n1 user PIN] "Goal"
|
+-- [n2 assistant] "Plan A"
|   |
|   [n3 assistant] "Tests passed" [HEAD* "main"]
`-- [n4 assistant] "Plan B"
    |
    [n5 assistant] "Experiment" [HEAD "alternative"]
```

- `|` 表示连续父子关系，`+--` / `` `-- `` 表示同一父节点的子树；只有分叉增加缩进，连续历史始终保持同一列。长链不需要不断横向滚动。
- 当前分支路径先显示，再显示其他普通历史，最后是摘要支路；同级用节点 ID 数字顺序稳定排列。共享前缀只显示一次，包含全部热节点与冷区节点。
- `HEAD* "分支名"` 标明当前分支 head，其他分支用 `HEAD`；同一节点可有多个 head 标记。`PIN`、`COLD`、`PLACEHOLDER`、`STALE` 分别表示固定、归档、待回填占位和已过时摘要。
- 摘要节点带版本 `vN`；下一行 `covers=n2..n3` 标明覆盖区间，`supersedes=n7` 标明替代旧版，`derived_from=n8,n9` 标明依赖。这些是关系注释，不是分支合并，也不替换原文节点。
- 消息、分支名均加引号；预览至多取原文前 64 个 Unicode 标量，超出附加 `...`。这不是模型 token 预算，也不从数据库删除文本。完整消息使用 `tl node --node ID`，完整分支名使用 `tl branches`。
- 换行、回车、制表符显示为 `\n`、`\r`、`\t`；终端控制字符和方向控制符显示为 `\u{...}`。没有 ANSI 颜色或 Unicode 画框字符，不执行原文中的控制序列。终端自动换行由终端宽度决定。

## 输出与只读边界

省略 `--output` 时 stdout 为 UTF-8 文本图；传入文件路径时 stdout 为 JSON，例如 `{"format":"ascii","timeline_id":"tl0","output":"task.txt","nodes":8,"edges":9}`。节点与关系计数同 draw.io：包含全部节点、父子边以及摘要覆盖端点、版本和依赖，关系注释中的单节点覆盖区间只计一次。错误仍为 stderr JSON 与非零退出码；参数、读取或渲染失败时不向 stdout 输出半幅图。

读取使用现有 `Storage::read`，只做一次快照；不调用 `context_budget`，不修改数据库、备份、计数器、审计、水位或 MCP。文本文件没有反向导入与同步；编辑它不会回写 Timeline。ASCII 是供人查看的预览，不作为模型上下文来源；给模型的上下文仍遵循 `context_budget` 契约。需要可拖动的图形时使用 [draw.io 格式](drawio.md)。

## 验证

`tests/ascii.rs` 覆盖确定性分支布局、共享前缀和多 head、冷热原文与摘要版本、占位只读性、中文及终端控制符、空历史、6000 条消息长链、stdout 与文件内容一致、拒绝覆盖、错误路径和下游提前关闭管道。draw.io 的原有回归同时运行，验证共用拓扑处理后行为保持一致。

```sh
cargo test --test ascii --test drawio --locked
python scripts/acceptance.py
```

可以直接查看通过 CLI 从合成对话生成的 [文本示例](../examples/timeline.ascii.txt)，与 draw.io 示例使用相同的数据，不包含真实会话。本次实际测试与 shell 验证记录见 [验收记录](ACCEPTANCE.md)。
