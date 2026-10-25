# 概述

kikoeru-neo 是一个自托管的有声剧和有声书播放器，从 DLsite 获取作品信息。它是独立重写的项目，可以读取旧 kikoeru-express 的数据格式。后端使用 Rust，前端使用 Svelte 5，源码、构建和测试都在本仓库维护。

这本中文设计书面向实现者，说明模块边界、数据与接口合同，以及测试和构建约束。修改功能前先读相关章节，涉及前后端字段时以[接口合同](contract.md)为准。

安装、配置和日常运行请从[中文使用指南](https://github.com/QWERChin/kikoeru-neo/blob/main/docs/zh-CN/index.md)开始，设计书不再重复操作步骤。

## 当前功能

- 浏览器和 PWA 提供作品浏览、搜索、收藏、评分、播放进度，以及音频和视频播放。
- 媒体来源包括本地目录、OpenList 和 WebDAV。同一部作品的多个位置合并显示，重复文件按[数据模型](data-model.md)的规则处理。
- 从旧 kikoeru-express 导入作品和用户状态，也可以只把用户状态合并进已有库，见[迁移](migration.md)。
- 音频支持波形和响度均衡，视频支持 HLS 回退和 ASS 字幕。
- 服务器提供 Nix 包、NixOS 模块和 Docker 镜像构建。Tauri 桌面应用支持连接服务器和本机媒体库两种模式。

## 范围与约束

不兼容旧前端产物、旧 REST API 或 socket.io 接口。数据库默认使用 SQLite，服务器也支持 PostgreSQL，不支持 MySQL。移动端使用网页和 PWA，不提供原生移动应用。歌词翻译任务、文件管理器和远端写操作不在当前功能范围内。

手机后台播放、锁屏控制和桌面 WebView 的验证范围见[测试与验证](testing.md)，尚未完成的工作见[路线图](roadmap.md)。

## 阅读顺序

先读[系统架构](architecture.md)了解模块边界，再读负责领域的设计。数据库改动需要同时核对[数据模型](data-model.md)，接口改动需要更新合同和合成样本。构建方法见[构建与部署](deployment.md)，稳定的产品选择见[设计决定与开放问题](open-questions.md)。

项目使用 GPL-3.0-or-later 许可证。
