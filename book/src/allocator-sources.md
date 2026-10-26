# 分配器版本与移植边界

这份记录用于解释[分配器测量](testing.md#分配器测量)的构建条件和适用范围。上游声明不能代替本项目的负载结果，也不能把 x86_64 的运行结果当作 aarch64 的验证。

## 比较的版本

`tikv-jemallocator 0.7.0` 依赖 `tikv-jemalloc-sys 0.7`。当前未撤回的 sys 发布版是 `0.7.1+5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8`，内置 jemalloc 5.3.1。crate 默认编译后台线程支持，但默认不启动后台线程，也不启用 `stats`。启用统计的诊断构建应单独标记，不能称为完全默认构建。[crate 功能定义](https://docs.rs/crate/tikv-jemallocator/0.7.0/source/Cargo.toml.orig)、[sys 版本与功能](https://docs.rs/crate/tikv-jemalloc-sys/0.7.1%2B5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8)

`mimalloc 0.1.52` 配合 `libmimalloc-sys 0.1.49` 时，默认已经是 mimalloc v3，显式启用 `v2` 才使用 v2。发布包分别内置 v3.3.2 和 v2.3.2。因此比较表必须写出实际版本和功能，不能继续把当前 crate 的默认值称为 v2。[crate 功能定义](https://docs.rs/crate/mimalloc/0.1.52/source/Cargo.toml.orig)、[发布包源码](https://docs.rs/crate/libmimalloc-sys/0.1.49/source/)、[v2 版本头](https://github.com/microsoft/mimalloc/blob/v2.3.2/include/mimalloc.h)、[v3 版本头](https://github.com/microsoft/mimalloc/blob/v3.3.2/include/mimalloc.h)

## 统计与回收

jemalloc 5.3.1 的 `dirty_decay_ms` 默认是 10000，`muzzy_decay_ms` 默认是 0。默认带前缀构建读取 `_RJEM_MALLOC_CONF`，不是不带前缀的 `MALLOC_CONF`。如果初测表明空闲回收落后，再比较后台回收或缩短 decay，必须保持其他构建条件相同。[5.3.1 手册源码](https://github.com/jemalloc/jemalloc/blob/5.3.1/doc/jemalloc.xml.in)、[前缀规则](https://github.com/jemalloc/jemalloc/blob/5.3.1/INSTALL.md#advanced-configuration)

启用 jemalloc 的 `stats` 后，可以推进 `epoch` 再读取 `stats.allocated`、`stats.active` 和 `stats.resident`。三个数分别描述分配器的在用分配、活动页和驻留估计，不等同于整个进程的 RSS。未覆盖的 C 分配、映射及线程栈仍应由 `/proc` 观察。[控制接口](https://docs.rs/tikv-jemalloc-ctl/0.7.0/tikv_jemalloc_ctl/)

mimalloc v2.3.2 和 v3.3.2 的普通 release 构建默认 `MI_STAT=0`。v3 提供 `mi_stats_get_json`，但接口存在不表示所有分配计数都已启用。v2 的 `mi_stats_merge` 合并调用线程的统计，不能由一个采样线程代替所有 Tokio 线程执行。`mi_process_info` 的 committed 或 RSS 也不能当成仍在使用的分配量。缺失的计数应写成未测得，不应填写零。冷却阶段不能调用 `mi_collect` 或其他强制回收接口，否则测到的不是自然回落。[v2 统计开关](https://github.com/microsoft/mimalloc/blob/v2.3.2/include/mimalloc/types.h)、[v3 统计开关](https://github.com/microsoft/mimalloc/blob/v3.3.2/include/mimalloc/types.h)、[Rust 扩展接口](https://docs.rs/crate/libmimalloc-sys/0.1.49/source/src/extended.rs)

仅设置 Rust 的 `#[global_allocator]` 不会自动替换 C 库的 malloc。jemalloc 的默认符号带前缀，mimalloc 的 `override` 也不是默认功能。SQLite 等 C 代码因此仍可能使用 libc malloc。外部 ffmpeg 和 ffprobe 进程也不会使用服务端的 Rust 全局分配器，但会继承 `MALLOC_ARENA_MAX` 等环境变量。因此服务器自身与媒体子进程的 CPU 时间需要分列，不能把子进程的变化都归因于 Rust 分配器。分配器覆盖范围和统计覆盖范围必须在报告中说明，不能把 Rust 堆统计当作进程全部分配。[jemalloc 覆盖功能](https://docs.rs/crate/tikv-jemalloc-sys/0.7.1%2B5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8)、[mimalloc 构建脚本](https://github.com/purpleprotocol/mimalloc_rust/blob/abcd2be6171e89190f8087fcaccf001d8bd5bc96/libmimalloc-sys/build.rs)

## 构建、页大小与维护

jemalloc 的 allocator page 至少要和运行系统的 page 一样大。若在 4 KiB 页机器构建 aarch64 通用包，再放到 16 KiB 或 64 KiB 页内核运行，默认探测值不够。`JEMALLOC_SYS_WITH_LG_PAGE=16` 对应 64 KiB，但会改变分配粒度，仍需分别在 4、16、64 KiB 内核验证启动和负载。当前项目静态包只发布 x86_64，这一风险主要约束未来的 aarch64 静态包以及可迁移的 aarch64 glibc 包。[jemalloc 页大小约束](https://github.com/jemalloc/jemalloc/blob/5.3.1/INSTALL.md#advanced-configuration)

mimalloc 的 Unix 初始化通过 `sysconf(_SC_PAGESIZE)` 取得运行系统页大小，避免了相同的构建时页大小假设。这个源码事实不证明本项目已经在 aarch64 的 16 KiB 或 64 KiB 页内核通过测试。[v2 初始化](https://github.com/microsoft/mimalloc/blob/v2.3.2/src/prim/unix/prim.c)、[v3 初始化](https://github.com/microsoft/mimalloc/blob/v3.3.2/src/prim/unix/prim.c)

Linux 上的 mimalloc Rust 绑定使用 `cc` 编译内置的 C 文件，不要求 CMake。jemalloc 绑定运行发布包附带的 configure 和 make，需要目标 C 工具链、shell、make 及常用构建工具。直接从未打包的 jemalloc 开发源码构建还涉及 autoconf。交叉编译时两者都需要正确的目标工具链。[mimalloc 构建脚本](https://github.com/purpleprotocol/mimalloc_rust/blob/abcd2be6171e89190f8087fcaccf001d8bd5bc96/libmimalloc-sys/build.rs)、[jemalloc 绑定构建脚本](https://docs.rs/crate/tikv-jemalloc-sys/0.7.1%2B5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8/source/build.rs)、[jemalloc 构建说明](https://github.com/jemalloc/jemalloc/blob/5.3.1/INSTALL.md)

本次 Nix 开发构建复现了 jemalloc 的 configure 探测失败。探测使用 `-Werror`，而 glibc 的 fortify 头文件在 `-O0` 下报警，最终表现为无法确定 `strerror_r` 的返回类型。同一个 C 探测在 `-O1` 下通过，因此 workspace 只为 `tikv-jemalloc-sys` 的 dev profile 设置 `opt-level = 1`，保留强化检查。这个设置不改变本次测量的 release profile。

核查时两个 Rust 包装仓库和两个 C 上游都未归档，不能沿用“jemalloc 已停止维护”的旧判断。jemalloc 上游已发布 5.4.0，而此处的 crate 仍内置 5.3.1。mimalloc 上游推荐 v3.6.0，并把 v2.5.2 列为继续接收修复的旧稳定分支，而 Rust crate 内置版本更早。选择任一绑定都需要跟踪其内置 C 版本与上游修复的差距，不能只看 Rust crate 是否更新。[jemalloc 发布记录](https://github.com/jemalloc/jemalloc/releases/tag/5.4.0)、[TiKV 包装层发布记录](https://github.com/tikv/jemallocator/releases/tag/0.7.0)、[mimalloc 版本策略](https://github.com/microsoft/mimalloc#versions)、[mimalloc Rust 包装层](https://github.com/purpleprotocol/mimalloc_rust)

## 采样边界

项目的 SQLite 写池最多一个连接，读池最多四个，没有覆盖 SQLx 的连接寿命参数。SQLx 0.9.0 默认在连接空闲十分钟或存活三十分钟后关闭连接，每个 SQLite 连接又有独立工作线程。因此，三十分钟附近的 RSS、线程数和文件描述符同时下降，可能来自连接退休和重建，不能直接归因于同一时间附近的扫描或分配器回收。这个解释需要连接关闭事件才能进一步确认。比较各变体时保留相同池设置，并同时看原始曲线和冷却阶段，而不是只比较跨越跳变的回归斜率。[池默认值](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/options.rs)、[连接归还](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/connection.rs)、[SQLite 工作线程](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-sqlite/src/connection/worker.rs)

`/proc/status` 的 `VmHWM` 也不是精确的单调计数。Linux 7.0 输出时会比较已保存高水位和当前 RSS 的求和结果，但保存高水位使用的是更快的近似计数。当前 RSS 回落后，显示的 `VmHWM` 因此也可能降低。报告分别保留所有样本的最大 RSS 和最大 `VmHWM`，不只读取结束时的高水位。三十秒采样仍可能漏掉瞬时峰值。[状态输出](https://github.com/torvalds/linux/blob/v7.0/fs/proc/task_mmu.c)、[RSS 计数与高水位](https://github.com/torvalds/linux/blob/v7.0/include/linux/mm.h)

观察回收时还必须读取 swap。本轮短诊断出现过非零 `VmSwap`，因此仅凭 RSS 下降不能证明分配器交还了空闲内存。后续确认运行同时记录 RSS、swap 和两者之和，未记录 swap 的旧数据保持未测状态，不补成零。RSS 加 swap 仍包含代码、栈等内存，也不是分配器在用字节数。

## 本轮使用的 mimalloc 选项

发布包中的 v2.3.2 默认 `MIMALLOC_PURGE_DELAY=10` 毫秒，arena 的延迟乘数为 10。v3.3.2 默认延迟是 1000 毫秒，arena 的乘数为 1。设置延迟为 0 会立即处理可回收的空闲范围，但不能释放仍含存活对象的页，并可能增加系统调用与缺页成本。延迟到期也不代表存在后台定时器保证执行回收。[v2 默认值](https://github.com/microsoft/mimalloc/blob/v2.3.2/src/options.c)、[v3 默认值](https://github.com/microsoft/mimalloc/blob/v3.3.2/src/options.c)、[arena 回收路径](https://github.com/microsoft/mimalloc/blob/v3.3.2/src/arena.c)

两版都默认允许 THP。`MIMALLOC_ALLOW_LARGE_OS_PAGES=0` 并不等于关闭透明大页。实际测量使用的 `MIMALLOC_ALLOW_THP=0` 会调用进程级 `prctl`，这个状态会由 ffmpeg 等子进程继承，没有修改主机全局设置。Cargo 的 `no_thp` 特性只抑制显式大页建议，不等价于这个运行时选项。`AnonHugePages` 可用于观察传统 PMD 大小的匿名透明大页，但不是所有多尺寸 THP 的完整计数。[mimalloc Unix 实现](https://github.com/microsoft/mimalloc/blob/v3.3.2/src/prim/unix/prim.c)、[内核进程级 THP 控制](https://www.kernel.org/doc/html/latest/admin-guide/mm/transhuge.html#process-thp-controls)
