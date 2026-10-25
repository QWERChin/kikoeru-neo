# 在 NixOS 上安装

NixOS 模块会创建服务用户、数据目录和 systemd 单元，并安装账号与备份命令。需要一台 `x86_64-linux` 或 `aarch64-linux` 的 NixOS 主机，以及修改和激活系统配置的权限。

## 加入系统配置

在现有系统 flake 的 `inputs` 中加入：

```nix
kikoeru-neo.url = "github:QWERChin/kikoeru-neo";
```

在 `outputs` 的参数中接收 `kikoeru-neo`，然后把以下内容加到目标系统的 `modules` 列表中。保留该输入自己的 nixpkgs，不需要添加 `inputs.nixpkgs.follows`。

```nix
kikoeru-neo.nixosModules.default
{
  services.kikoeru-neo = {
    enable = true;
    settings.listen = "127.0.0.1:8888";
  };
}
```

`flake.lock` 固定实际使用的源码版本，保存它以便重建。上面的配置使用 SQLite，数据在 `/var/lib/kikoeru-neo`，运行用户和组都是 `kikoeru-neo`。模块选项见[配置](configuration.md)。

## 激活、创建管理员、打开页面

在系统配置所在目录，使用你现有的目标名称激活。例如目标名是 `media` 时：

```sh
sudo nixos-rebuild switch --flake .#media
systemctl status kikoeru-neo
sudo -u kikoeru-neo kikoeru-neo user create --admin admin
```

先激活再创建账号，因为激活才会创建系统用户并安装包装后的命令。密码交互输入两次，至少 8 个字符。成功后输出 `created admin admin`。

在服务器本机打开 `http://127.0.0.1:8888`。没有本机浏览器时，可以从管理电脑转发端口，以下 `media.example.lan` 代表自己的服务器地址：

```sh
ssh -L 8888:127.0.0.1:8888 media.example.lan
```

保持 SSH 连接，在管理电脑打开同一回环地址。若本机端口已被占用，换一个本地端口。登录后按[媒体库与扫描](libraries.md)添加媒体库并完成首次扫描。

## 文件权限和配置

本地媒体目录应让 `kikoeru-neo` 用户有目录遍历权限和文件读取权限。可以通过已有的媒体组或 ACL 授予只读权限，不要把应用数据目录改成同组可写。模块通过 systemd 把媒体文件系统设为只读，只有数据目录可写。

`settings` 会写进所有本机用户都能读取的 Nix store，因此不要在其中写密码。界面中添加的远端密码存在数据目录里。声明式媒体库使用 `password_file`，由自己的密钥管理工具提供文件，见[媒体库](libraries.md)。

模块安装的 `kikoeru-neo` 默认使用服务的配置和数据目录。账号、备份等命令要以服务用户运行。`--data-dir` 不会取消配置中的 PostgreSQL 地址，操作恢复副本时必须使用[隔离配置](backup-restore.md)。

## PostgreSQL

新安装可以加入：

```nix
services.kikoeru-neo.database.createLocally = true;
```

模块启用本机 PostgreSQL，创建与服务用户同名的角色和数据库，通过 `/run/postgresql` 的 Unix socket 和 peer 认证连接，端口跟随 PostgreSQL 设置。已有 SQLite 数据时，不要直接打开此选项就启动空库，先按[数据库复制](configuration.md#更换数据库)完成备份、停服和复制。

## 日常检查与限制

```sh
journalctl -u kikoeru-neo -n 100 --no-pager
journalctl -u kikoeru-neo -f
```

`systemctl` 应显示服务运行，`http://127.0.0.1:8888/api/ready` 应返回 HTTP 200。没有管理员时就绪接口也会成功，媒体库是否可访问要登录后另查。

模块不配置防火墙、DNS、证书或反向代理。跨机器访问按[安全与网络](security.md)设置入口。变更配置后重新激活系统会重启服务，先确认没有需要保留的运行中扫描，见[升级](upgrade.md)。
