# 快速开始

在 Linux 上用 Nix 构建后，可以直接创建管理员、启动服务并扫描一个媒体库，不必先配置系统服务。服务器 Nix 包支持 `x86_64-linux` 和 `aarch64-linux`。需要长期运行时，转到 [NixOS](install-nixos.md) 或 [Docker](install-docker.md) 安装。

## 准备

安装支持 flakes 的 Nix 和 Git。准备一份自己有权读取的媒体目录，作品文件夹必须包含 `RJ`、`BJ` 或 `VJ` 加 6 到 8 位数字，例如 `RJ01234567 示例作品`。这只是合成编号示例，不表示线上存在对应作品，详细规则见[媒体库](libraries.md)。

## 构建并创建账号

```sh
git clone https://github.com/QWERChin/kikoeru-neo.git
cd kikoeru-neo
nix build .#neo-server
```

在同一个终端中准备独立配置。以下目录由当前用户拥有，不要以 root 运行试用实例：

```sh
umask 077
mkdir -p demo-data
printf 'listen = "127.0.0.1:8888"\n' > demo-data/config.toml
export KIKOERU_DATA_DIR="$PWD/demo-data"
export KIKOERU_CONFIG="$PWD/demo-data/config.toml"
unset KIKOERU_DATABASE_URL KIKOERU_LISTEN
./result/bin/kikoeru-neo user create --admin admin
./result/bin/kikoeru-neo serve
```

创建账号时输入两次密码，至少 8 个字符。成功后会输出 `created admin admin`。服务在前台运行，日志写到终端。构建包已提供 ffmpeg 和 ffprobe，不需要另装。

## 扫描并播放

1. 在这台机器的浏览器打开 `http://127.0.0.1:8888`，用刚建的账号登录。
2. 进入“设置”，点“添加媒体库”，选择本地类型，填写服务器上的绝对路径，例如 `/srv/media/works`。当前用户需要能列出目录并读取文件。
3. 保存后点该媒体库的“扫描”。任务一次运行一个，离开设置页不会取消扫描。
4. 回到“资料库”，打开作品，选择轨道播放。先确认能发声、拖动进度，再刷新页面核对进度。

本地音视频的时长探测单独排队，扫描结束后可能还没完成。抓不到元数据时，作品仍以目录名入库并可播放。没有作品出现时，先查[目录命名和扫描限制](libraries.md)，不要反复重建数据目录。

## 停止与继续使用

在服务终端按 Ctrl-C 停止。下次在同一检出目录运行：

```sh
KIKOERU_CONFIG="$PWD/demo-data/config.toml" \
  ./result/bin/kikoeru-neo --data-dir "$PWD/demo-data" serve
```

不要删除 `demo-data`，它保存账号、封面和进度。结束试用终端时，环境变量随之消失。需要在另一台设备访问时，按[安全与网络](security.md)配置监听、防火墙及 HTTPS，不要只把回环地址换成公网监听。

试用构建跟随克隆时的源码。正式安装应[固定版本并备份](upgrade.md)。
