# 管理账号

服务器不提供匿名访问或网页注册。创建、删除用户和创建管理员通过本地命令完成，普通用户可以在网页修改自己的密码或退出所有设备。运行命令前先确认它使用的是服务正在使用的配置和数据库。

## 创建第一个管理员

按 [Docker 安装页](install-docker.md)配置 Compose 后，在部署目录运行：

```sh
docker compose run --rm kikoeru-neo user create --admin admin
```

[单文件版](install-binary.md#作为系统服务运行)作为系统服务运行时，使用那里定义的 `kikoeru_neo` 函数。它以服务用户身份、用服务的配置运行命令：

```sh
kikoeru_neo user create --admin admin
```

NixOS 模块激活后运行：

```sh
sudo -u kikoeru-neo kikoeru-neo user create --admin admin
```

交互输入两次密码，成功后输出 `created admin admin`。然后打开服务页面登录。密码至少 8 个字符，用户名为 1 到 64 个字符，不能有控制字符或首尾空白。选择足够长且不复用的密码。

## 添加、重设和删除

下表列出子命令，继续使用上面适合自己安装方式的命令前缀。`reader` 是合成账号名。

| 子命令 | 结果 |
|---|---|
| `user create reader` | 创建普通用户，交互设置密码。 |
| `user create --admin operator` | 创建另一个管理员。 |
| `user set-password reader` | 重设密码，并撤销该用户已有的全部 token。 |
| `user delete reader` | 删除用户并使其 token 失效，最后一个管理员不能删除。 |

没有修改现有用户管理员身份的命令或网页接口。不要把“创建同名管理员”当作提权方法，同名账号会被拒绝。

脚本可以对 `create`、`set-password` 加 `--password-stdin`，从标准输入第一行读密码。例如已有仅自己可读的 `admin.pw` 时：

```sh
docker compose run --rm -T kikoeru-neo user set-password admin --password-stdin < admin.pw
```

命令成功后按自己的凭据保存规则处理临时密码文件。不要把密码放在命令参数、shell 历史或公开日志中。

## 网页修改密码与退出

在“设置”的账号区域修改密码时，需要当前密码。修改成功后旧登录全部失效，需要重新登录。“退出所有设备”同样撤销该用户的 API token 和媒体 token，只退出当前设备则只清除当前页面保存的登录。

普通用户可以浏览所有媒体库并播放，自己的收藏、评价和进度独立保存。管理员额外能够配置媒体库、扫描和编辑元数据。当前没有按用户隔离媒体库的权限设置。

## 导入账号和常见问题

旧 kikoeru-express 的密码不导入，导入的账号必须用 `user set-password` 设置新密码。合并时新建的账号没有密码且不是管理员，已有账号的身份不改，见[迁移](migration.md)。

`/api/ready` 成功不代表已经创建管理员。登录页提示尚未初始化时，执行创建管理员命令。命令显示成功而网页仍然要求初始化，优先核对配置文件、数据目录和 PostgreSQL 地址，见[故障排查](troubleshooting.md)。

桌面本机模式自动创建本机管理员并登录，不需要照抄本页的服务器初始化步骤，见[桌面应用](desktop.md)。
