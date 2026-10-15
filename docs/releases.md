# 发布二进制版本

用户从 [Releases](https://github.com/oonecab/timeline/releases) 下载已发布的版本。维护者推送一个新 tag 后，[Release 工作流](../.github/workflows/release.yml) 会检查并构建四个平台，全部成功后创建或更新与 tag 同名的 GitHub Release，并自动生成发布说明。

## 什么时候会发布

工作流监听 `push.tags: ['**']`，因此任意 tag 都会触发，包括 `release/v1.1.2` 这样带 `/` 的名称。不要求 tag 以 `v` 开头，也不会根据名称自动判断预发布版本：所有 tag 都创建普通 Release，`-beta`、`-rc` 等后缀也不例外。

只在本地创建 tag 不会触发发布；把包含工作流的代码推到分支，也不会自动发布。工作流生效后的 tag 推送才触发运行，不会追溯处理远端已有 tag。tag 必须指向要发布的提交，而且该提交中应包含 `.github/workflows/release.yml`。

`tl --version`、`tl-mcp --version` 和 MCP 握手中的 `serverInfo.version` 都在构建时从 Cargo 包版本自动读取。tag 只是发布标识，不会修改包版本。发布前应确认 `Cargo.toml` 的包版本与 `Cargo.lock` 中 `timeline` 条目的版本一致；需要升级版本时，先同步这两个文件、完成检查并提交，再给目标提交打 tag。

## 推送一个新版本

下面以下一次版本 `v1.1.2` 为例，请替换为实际要发布且尚未使用的版本。命令可以在 PowerShell、bash 或 zsh 中运行。

先确认工作区、目标提交和远端地址，并检查这个 tag 是否已被使用：

```sh
git status --short
git log -1 --oneline
git remote -v
git show HEAD:.github/workflows/release.yml
git tag --list v1.1.2
git ls-remote --tags origin refs/tags/v1.1.2
```

`git status --short` 应没有待发布的未提交改动；后两条查询应没有同名 tag 的输出。确认 `HEAD` 就是要发布的提交，本地检查已通过，再创建带说明的 tag，并只推送这一个 tag：

```sh
git tag -a v1.1.2 HEAD -m "Release v1.1.2"
git push origin refs/tags/v1.1.2
```

这些命令会发布所选提交，请在版本准备好时执行。不要为了补发版本移动或覆盖旧 tag，也不要一次推送全部 tag；如果名称已使用，选一个新的版本名。

推送后打开 [Release Actions](https://github.com/oonecab/timeline/actions/workflows/release.yml)，查看该 tag 的运行。四个平台的构建和 `publish` job 都成功后，再到 [Releases](https://github.com/oonecab/timeline/releases) 确认发布说明和四个资产。工作流文件存在只代表已配置发布流程，实际发布状态以运行和 Release 页面为准。

## 四平台检查与发布资产

`ubuntu-latest`、`macos-latest`、`macos-15-intel`、`windows-latest` 都运行同样的检查和构建命令：

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cargo build --release --locked
```

Intel Mac 会在自己的 runner 上完整执行测试。四个平台全部成功后，单独的 `publish` job 才会创建或更新 Release；任何一个平台失败，都不会进入发布步骤。

资产名称、CPU 架构与下载后的启动方式见 [README 的下载说明](../README.md#下载与开始使用)。Windows 使用 ZIP；Linux/macOS 使用保留执行权限的 `.tar.gz`。每个包的根目录只有 `tl` 和 `tl-mcp`，Windows 对应为 `tl.exe` 和 `tl-mcp.exe`。正常解压 Release 的 `.tar.gz` 后无需额外 `chmod`；Actions artifact ZIP 的权限处理仍按 README 执行。

发布使用与 CI 相同的原生 runner，不扩大系统兼容范围。Intel runner 的生命周期和其他兼容限制见 [运行环境](environments.md#ci-平台与兼容范围)。

## 重跑与权限

如果构建或发布失败，先从 Actions 日志确认原因。重新运行同一 tag 的工作流，会使用该 tag 对应的提交；普通 Release 已存在时会更新它，并覆盖同名资产。重跑不是一个新版本，已有下载可能与重新生成的文件不同。

GitHub 不允许覆盖已发布的不可变 Release。需要修正内容时发布新的 tag，不要移动原 tag 或尝试替换不可变资产。

认证使用 GitHub 自动提供的 `GITHUB_TOKEN`。构建 job 保持只读权限，仅 `publish` job 使用 `contents: write` 来创建 Release 和上传资产。不需要新建个人令牌、手工 secret，也不需要给本机添加发布环境变量。应用运行时仍只使用 [环境说明](environments.md#数据库路径) 中的变量。
