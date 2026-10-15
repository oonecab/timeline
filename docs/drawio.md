# draw.io 文件导出

`tl export` 把一条 Timeline 导出为可手动编辑的 `.drawio` 文件。它是 CLI 的只读扩展，不属于 v1.1.2 的 14 个 MCP 工具；现有 ADT 契约、数据库格式与 stdio 协议保持不变。本文件定义导出行为与展示规则。

## 使用

```sh
tl --db conversation.timeline.json --timeline tl0 export --format drawio --output task.drawio
```

- `--output` 必填，父目录必须已存在；推荐使用 `.drawio` 扩展名。
- `--format drawio` 可以省略，仍为默认格式；终端文本图使用另一种格式 `ascii`，见 [终端图说明](ascii.md)。
- `--db`、`TIMELINE_DB` 与默认用户路径的优先级沿用现有 CLI；省略 `--timeline` 时选择 ID 数字最小的时间线。
- `export` 不接受 `--idempotency-key`。导出不会进入变更事务或写幂等记录。
- 成功后 stdout 输出一个 JSON 对象，例如 `{"format":"drawio","timeline_id":"tl0","output":"task.drawio","nodes":8,"edges":9}`；计数包含隐藏图层的关联边，不含说明文字和图层容器。
- 已有目标文件始终拒绝覆盖，包含数据库本身、备份和链接。重复导出请用新文件名，例如 `task-v2.drawio`，以保留人工布局和批注。
- 读取失败、时间线不存在、格式不支持或文件写入失败时返回非零退出码和 JSON 错误，不报告成功。

无需安装绘图软件即可执行导出。查看和编辑时使用 draw.io：从其文件菜单打开 `.drawio` 文件；已关联该扩展名的桌面环境也可直接双击。节点和连线都是原生图形，可拖动、改大小、双击修改标签、调整连线并另存。查看完整原文和元数据，可选中节点后使用 **编辑数据（Edit Data）**。draw.io 桌面版支持离线编辑，见[官方说明](https://www.drawio.com/docs/security/desktop-secure-diagrams/)。

[examples/timeline.drawio](../examples/timeline.drawio) 是通过 CLI 从合成对话生成的示例，不含用户实际会话。

## 图形含义与布局

- 一条时间线生成一页，包含根、全部热节点和冷区节点；共享前缀只画一次。已删除的分支指针不会被虚构出来，仍保留的历史节点照常展示。
- 历史父子关系从左向右，当前分支路径优先沿同一行展开；其他子树各占独立行，普通历史优先于摘要支路。同一深度对齐，节点 ID 按数字顺序稳定排列，重复导出同一状态得到相同文件。
- 普通消息使用浅蓝圆角矩形；当前分支 head 为绿色；冷区节点为灰色；摘要为浅紫色。标签保留节点 ID、角色和短预览，按需标注 `PIN`、`冷区`、`待回填`、`STALE`、摘要版本、覆盖区间及分支 head。颜色以当前 head 优先。
- 横轴表示历史层级，不表示真实时间；横纵坐标只属于图文件，不存回数据库。手动调整后按用户在 draw.io 中保存的布局展示。
- 图中摘要仍是原有独立节点，原文没有被替换；摘要的真实 parent 可以位于冷区。

文件包含以下图层，可通过 draw.io 的图层面板（通常位于 **查看 → 图层**）控制显示：

| 图层 | 默认显示 | 内容 |
|---|---|---|
| 历史节点与父子关系 | 是 | 节点和灰色实线箭头，严格对应 `parent` |
| 摘要覆盖（非历史父子关系） | 否 | 从摘要指向覆盖起点、终点的虚线；单节点区间只画一条 |
| 摘要版本与依赖（非分支合并） | 否 | 新摘要指向被替代旧版、依赖摘要的虚线，分别标注“替代旧版”和“参考摘要” |

覆盖边只标记区间边界，完整区间保存在节点数据中；版本与依赖边不是 merge，也不改变历史父子关系。

## 原文与快照边界

每个节点保存原始 ID、角色、tokens、parent、children、所在冷热区、pin、分支 head、摘要区间、版本、替代关系、依赖、stale 和占位状态。完整消息在 `text_json` 中，以 JSON 字符串保存；换行等字符使用 JSON 转义，解码后可恢复原文。多值字段使用 `children_json`、`branches_json`、`summary_range_json`、`derived_from_json`。

节点标签只是有限长度的展示预览。标签使用纯文本，不执行消息中的 HTML 或占位表达式；XML 属性会转义引号、尖括号和控制字符。即使消息包含 XML 1.0 不支持的字符，`text_json` 仍保留可恢复的原文。导出包含冷热区原文，文件的内容范围与所选时间线一致。

导出仅通过 `Storage::read` 获得一次已校验的快照，沿用现有共享锁；不会修改数据库、备份、节点、分支、水位、计数器、审计或幂等结果，也不会调用可能创建占位的 `context_budget`。输出先写入目标目录的临时文件，再以不覆盖已有文件的方式保存。

图文件与原数据库没有实时连接。新增会话内容后需重新导出；在图上改文字、连线或节点位置只改变图文件。没有导入、反向同步或网页服务。给模型组装上下文仍使用原有 `context_budget` 契约，不从用户手改后的图推导历史。

## 文件结构与验证

文件使用未压缩的 `mxfile → diagram → mxGraphModel` XML，节点和边为带 `UserObject` 元数据的原生 `mxCell`，布局在 `mxGeometry` 中；依据 [draw.io 官方文件格式](https://github.com/jgraph/drawio/wiki/File-Format)。无需运行时 XML、图布局或浏览器依赖。

`tests/drawio.rs` 使用仅供测试的 `roxmltree` 解析器验证 XML 和节点/连线契约，并覆盖确定性布局、冷热关联、摘要版本、特殊字符、长消息、6000 条消息的线性历史、CLI 只读性、拒绝覆盖、输出竞争及错误路径。Unix 另有链接目标保护回归。执行：

```sh
cargo test --test drawio --locked
python scripts/acceptance.py
```

实际执行结果及官方渲染器的打开、移动、改名、图层切换和保存验证见 [验收记录](ACCEPTANCE.md)。
