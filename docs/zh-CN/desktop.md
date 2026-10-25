# 使用桌面应用

桌面应用提供独立窗口，可以连接已有服务器，也可以在个人电脑上运行本机媒体库。先选择适合系统的安装包，再选择模式。界面与网页版相同，目前为中文。

## 获取应用

macOS 和 Windows 安装包由仓库的 [Desktop 工作流](https://github.com/QWERChin/kikoeru-neo/actions/workflows/desktop.yml)生成。打开对应版本的成功运行，在页面底部的 Artifacts 中选择：

| 系统 | 产物名称 | 内容 |
|---|---|---|
| Apple Silicon Mac | `kikoeru-neo-aarch64-apple-darwin` | 包含 `.app` 的 `.dmg` 磁盘映像。 |
| Windows x86_64 | `kikoeru-neo-x86_64-pc-windows-msvc` | NSIS 安装程序。 |

工作流把安装包保留 30 天，没有把它们自动发布到 GitHub Releases。下载通常需要登录 GitHub，过期或没有生成安装包的运行不会提供可用下载。本页不保证当前存在某个版本的产物。Intel Mac 没有该工作流的安装包。

Linux 可以在有图形会话的机器上从源码构建，Nix 包支持 `x86_64-linux` 和 `aarch64-linux`：

```sh
nix build .#desktop
./result/bin/kikoeru-neo-desktop
```

包已包含所需 ffmpeg 及 WebKitGTK 媒体依赖。Linux 桌面支持仍有验证缺口，不能以构建成功代替播放验证。

安装包没有开发者证书签名，macOS 只使用 ad-hoc 签名，系统可能阻止首次打开。核对下载来自本项目的目标运行后，按当前系统的应用安全提示处理。不要全局关闭系统的应用验证。自动更新尚未实现，更换安装包前先[备份](backup-restore.md)并从托盘完全退出旧应用。

## 连接服务器

1. 选择“连接服务器”，输入自己服务器的 HTTP 或 HTTPS 地址，例如 `https://media.example.lan`，替换为实际地址。
2. 应用检查服务器后打开其页面，使用服务器账号登录。
3. 打开一部作品播放，检查暂停、切轨和进度保存。

此模式加载服务器提供的前端，不把服务器数据库复制到电脑。服务器地址、网络和证书必须能从这台电脑访问。纯 HTTP 的限制和浏览器相同，见[安全与网络](security.md)。连错服务器时可从登录页或设置页“切换模式”，不必先成功登录。

## 本机媒体库

1. 选择“使用本机媒体库”。应用启动内置后端并自动登录，不用自己创建管理员。
2. 在设置里添加本地媒体库，用文件夹选择器选包含作品目录的根目录，再扫描。目录编号要求见[媒体库](libraries.md)。也可以添加 OpenList 或 WebDAV。
3. 在资料库打开作品播放。本机目录默认监听变化，定时扫描仍需自行启用。

本机后端只监听 `127.0.0.1` 的随机端口，只使用 SQLite，不是向其他设备提供固定端口的服务器。它忽略配置里的 PostgreSQL 地址和监听地址。要供其他设备访问，请安装独立服务器。

从旧 kikoeru-express 导入时，在本机模式的“设置 → 桌面应用”选择旧数据目录，先看预检报告，再选择合并到本机账号的旧用户和可添加的本地媒体库。图形导入只合并收藏、评价和进度，作品仍靠扫描生成。OpenList 库需自己添加，详细限制见[迁移](migration.md)。

桌面 HTTP 客户端不读取 macOS 或 Windows 的系统代理。只需元数据走代理时，在设置页“网络代理”填写应用所在电脑能访问的代理地址，保存并检查连通性。它不代理媒体文件。

## 关闭窗口、退出和切换

关闭窗口只会隐藏窗口，播放继续。通过托盘菜单“退出”才结束应用和本机后端。再次打开应用会显示已有窗口，不会运行第二个本机实例。

设置中的“切换模式”返回选择页。本机模式异常进入登录页时，从托盘退出后重开，让外壳重新签发登录凭据，不要尝试猜本机账号的随机密码。

## 数据和日志在哪里

下表是应用实际使用的路径，其中环境变量代表当前登录用户的系统目录，不是维护者的个人路径。

| 系统 | 本机媒体库数据 | 日志 |
|---|---|---|
| macOS | `$HOME/Library/Application Support/kikoeru-neo` | `$HOME/Library/Logs/io.github.qwerchin.kikoeru-neo` |
| Windows | `%APPDATA%\kikoeru-neo` | `%LOCALAPPDATA%\io.github.qwerchin.kikoeru-neo\logs` |
| Linux | `$XDG_DATA_HOME/kikoeru-neo`，未设置或非绝对路径时为 `$HOME/.local/share/kikoeru-neo` | `$XDG_DATA_HOME/io.github.qwerchin.kikoeru-neo/logs`，未设置时为 `$HOME/.local/share/io.github.qwerchin.kikoeru-neo/logs` |

数据目录结构与服务器一致，保存账号、数据库、凭据、封面、字幕和缓存。原始媒体仍在添加时选择的文件夹，不会搬进数据目录。模式和服务器地址另存于系统应用配置目录中、应用标识对应目录下的 `desktop.json`，不是这个媒体数据目录。

本机设置页和启动失败的选择页有“打开日志文件夹”。日志按 UTC 日期写成 `kikoeru-neo.<日期>.log`，打开日志写入器时只保留最新 7 个文件。默认级别为 `info`，可用 `RUST_LOG` 覆盖，数据配置里的 `log.level` 对桌面外壳不起作用。连接服务器模式不提供远程页面打开本机日志文件夹的权限。

备份前从托盘退出应用，再复制完整数据目录，或使用相同版本的服务端备份命令显式指定该数据目录及独立 SQLite 配置。需要保留外壳设置时另存 `desktop.json`。恢复不要覆盖仍在运行的应用，见[备份与恢复](backup-restore.md)。

## 验证范围与限制

仓库提供以下自动检查，不代表每份下载产物都已在自己的设备上验证。使用前应查看所选工作流运行的结果。

| 范围 | 已有检查方式 | 不能据此证明的行为 |
|---|---|---|
| macOS、Windows 安装包 | 安装产物冒烟检查，运行打包工具、内置服务和合成媒体。 | 冒烟不打开播放器窗口，不证明真实音频输出、媒体键或休眠恢复。 |
| Windows WebView2 | 工作流有本机和服务器模式的 WebDriver 测试，检查页面、播放及 CSP。 | 不等于全套 Windows Rust 测试或所有驱动和媒体格式验证。 |
| Linux WebKitGTK | Nix 构建和可单独运行的 WebDriver 测试。 | Linux WebDriver 不在 Desktop 工作流中。 |
| macOS WKWebView | 包构建、冒烟及本机模式启动检查。 | 没有同等的 tauri-driver 覆盖，视频、HLS、画中画和字幕仍需真机验证。 |

后台播放、系统媒体控制、休眠、关闭窗口时的进度，以及不同 WebView 的媒体兼容性都要在目标设备检查。Windows 当前不测量缓存盘剩余空间，`cache.min_free_bytes` 的检查跳过，仍有缓存总大小限制。取消转码时直接结束 ffmpeg，没有 Unix 上的终止信号宽限期。本机文件已按路径交给 Windows ffmpeg，但路径重开和特殊字符行为仍需真机验证。
