# 运行环境与数据库路径

Timeline 的 CLI 和 MCP 服务都是本机二进制。运行时不需要 Rust、Python 或外部服务，也不访问网络、不监听端口。`tl-mcp` 由 MCP 客户端启动，通过标准输入输出通信。

## CI 平台与兼容范围

从 [Actions](https://github.com/oonecab/timeline/actions/workflows/ci.yml) 中选择一次成功的 CI 运行，下载对应系统和 CPU 架构的 artifact。Windows 包含 `tl.exe`、`tl-mcp.exe`；Linux/macOS 包含 `tl`、`tl-mcp`。下载步骤和各平台包名见 [README](../README.md)。

Linux/macOS 解压 ZIP 后需要恢复执行权限，在二进制所在目录运行：

```sh
chmod +x tl tl-mcp
./tl --help
```

Windows PowerShell 在解压目录运行 `./tl.exe --help` 即可。若目录没有加入 `PATH`，使用 `./`、PowerShell 的 `.\` 或完整路径调用；MCP 配置建议始终使用二进制的绝对路径。

CI 产物面向对应 runner 的系统和架构：Apple Silicon 与 Intel Mac 使用不同的包。Intel 的 `macos-15-intel` runner 当前可用，但已有生命周期公告。[GitHub 曾公告该标签提供至 2027 年 8 月](https://github.com/actions/runner-images/issues/13045)，后续可用性以最新 runner 公告为准，不应依赖这一标签长期存在。包可以下载不等于兼容所有旧系统或 Linux 发行版；CI 验证范围见 [验收记录](ACCEPTANCE.md#跨平台-ci)。

## 数据库路径

CLI 与 `tl-mcp` 使用同一套路径规则，优先级为 **`--db PATH` > 非空 `TIMELINE_DB` > 用户目录下的 `.timeline.json`**。

| 环境变量 | 用途 | 缺省行为 |
|---|---|---|
| `TIMELINE_DB` | CLI/MCP 共用的 JSON 数据库路径 | 未设置或空值时使用用户目录下的 `.timeline.json` |
| `USERPROFILE` | Windows 用户目录，由系统提供 | 缺失时回退到 `HOME`；Linux/macOS 不读取 |
| `HOME` | Linux/macOS 用户目录，也是 Windows 的回退目录 | 没有可用用户目录时，须提供 `--db` 或非空 `TIMELINE_DB` |

默认路径通常为 Windows 的 `C:\Users\用户名\.timeline.json`、Linux 的 `/home/用户名/.timeline.json` 或 macOS 的 `/Users/用户名/.timeline.json`，具体以系统环境变量为准。

父目录必须已存在；路径支持空格和 Unicode，显式传入空的 `--db` 会报错。相对路径以进程的工作目录为基准。程序不展开路径中的 `~`、`$HOME` 或 `%USERPROFILE%`，也不会自动加载 `.env`。在 shell 中可由 shell 展开变量；MCP 配置里的 `command` 和 `args` 应填实际绝对路径，避免依赖客户端的启动目录或 shell 展开。

[`.env.example`](../.env.example) 只列出可用变量名。不要用空值覆盖系统提供的 `USERPROFILE` 或 `HOME`。在终端设置的变量只对该终端及其子进程生效；从桌面启动的 MCP 客户端不一定继承它们，可在客户端配置中显式设置 `env` 或 `--db`。

### Windows PowerShell

以下示例在用户目录创建数据文件夹。假定当前目录是解压后的二进制目录：

```powershell
$dataDir = Join-Path $env:USERPROFILE 'timeline-data'
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
$env:TIMELINE_DB = Join-Path $dataDir 'conversation.timeline.json'
.\tl.exe create
```

也可只为某次调用指定路径：

```powershell
.\tl.exe --db "$dataDir\another.timeline.json" create
```

### Linux/macOS（bash 或 zsh）

```sh
mkdir -p "$HOME/.local/share/timeline"
export TIMELINE_DB="$HOME/.local/share/timeline/conversation.timeline.json"
./tl create
```

单次调用的路径覆盖方式相同：

```sh
./tl --db "$HOME/.local/share/timeline/another.timeline.json" create
```

需要让 CLI 和 MCP 读取同一份对话时，给两者配置相同的数据库绝对路径。完整 MCP 配置示例见 [README](../README.md)。

## 数据文件与恢复

| 文件 | 用途 |
|---|---|
| `<db>` | 当前完整快照 |
| `<db>.lock` | 稳定的跨进程锁 |
| `<db>.bak` | 上一份快照 |
| 同目录的 `.timeline-*` | 事务写入时的临时快照 |

每次成功变更都会保存完整快照，未启用增量日志或后台定时器。临时文件在正常完成或返回错误时清理；进程异常终止留下的临时文件不参与加载。

数据库与备份包含对话、摘要、审计和幂等结果，未加密。请放在可信的本地目录，不要提交真实内容。Linux/macOS 新建文件的权限为 `0600`；Windows 在创建时设置仅所有者可访问的受保护 DACL。这些规则用于程序新建的文件，不代表从别处复制来的文件会自动获得相同权限。

恢复前停止所有使用该数据库的进程，并保留损坏文件，再从 `.bak` 显式恢复。程序不会静默覆盖损坏源数据。具体步骤见 [README 的审计与恢复说明](../README.md#审计与恢复)。

## 从源码构建与验收

只有从源码构建时才需要 Rust/Cargo；下载依赖可能需要网络。Cargo 声明最低 Rust 1.85，CI 使用 stable 工具链，最低版本尚未单独验证。Python 3 仅用于可选的验收、治理脚本，不是应用运行依赖。

本地验收脚本在 Windows 可用 `python scripts/acceptance.py`，在 Linux/macOS 通常用 `python3 scripts/acceptance.py`。构建出的文件位于 `target/release/`，Windows 带 `.exe` 后缀，Linux/macOS 不带后缀。

已执行的平台检查、历史工具链版本与测试证据见 [ACCEPTANCE.md](ACCEPTANCE.md)。机器可读清单见 [`.project-governance.json`](../.project-governance.json)，技术边界见 [`TECH.md`](../TECH.md)。
