<!-- Owner: src/main.rs, src/store.rs, src/http.rs, src/scoring.rs, src/ingestion.rs, src/push.rs, src/protocol.rs, src/policy.rs, src/legacy_validation.rs, src/admin.rs, src/operations.rs, src/metrics.rs, scripts/check-client.mjs, scripts/check-admin.mjs, src/assets.rs, scripts/pack-release.mjs, scripts/migrate-storage.mjs, web/index.html, src/forum.rs, src/presentation.rs -->
# terrarium-server

生态箱独立服务端，生产 HTTP/WS 与存储使用 Rust + SQLite。兼容现有 World 的 HTTP/WS 协议与 TS 存档，可通过 Native 原子发布控制器运行。公共接口仅监听回环地址，管理台使用独立私网端口。

已实现：

- 排行榜、摘要、分页观察页、单箱状态与按需回放读取。历史发布资源按 release ID 读取，打开的页面保留对应版本的渲染资源。
- 自动分配 ID/token，兼容旧 ID 注册，Bearer 鉴权，HTTP 完整上报和 WS 完整/增量上报。提交上限 8 MiB，每连接最多 2 个待处理报告，全局原始报告缓冲 24 MiB、同时验证最多 4 个报告，WS 每 30 秒检查心跳。
- 与 TS 一致的规范 JSON SHA-256、基线 revision/hash 和目标 hash 校验。首次连接、重连及校验失败要求完整状态；成功确认在 SQLite 事务提交后发送。
- 最低客户端版本、HTTP Push 开关及定时生效规则，忽略提交时返回 `accepted: false`。协议升级提醒使用独立记录，保留既有通知 ID，不进入管理员公告列表；支持公开公告及鉴权公告轮询。
- 独立私网管理端口，包含管理页面、MiSans 和 Lucide 资源；校验实际来源 IP、CIDR、Host 与 Origin，管理员 Bearer token 与普通箱子凭据分离。公共端口不提供管理接口。
- 公告发布、撤销、定向投递；反馈提交、分页、限流和处理标记；云控立即或定时变更。公告与自动协议提醒共用事务分配游标。
- 上报支持可选 `clientSystem: { name, version }`，记录 World 所在机器的操作系统与版本；完整、增量和 HTTP 上报均校验并持久保存。旧客户端可省略，私网管理台显示“未上报”；不改变数据库结构或 data epoch。
- 管理概览通过 SQL 提取箱子摘要，提供当前 WS 连接数、成功提交计数、运行时间及 Rust 主进程 RSS；不包含 Node 校验子进程内存。计数在重启后归零。
- SQLite WAL，持久写连接、独立读连接、`synchronous=FULL`。每箱一行，比较 revision 后更新；运行时数据库操作在阻塞线程执行。
- 旧 JSON 导入、导出、完整性检查、在线备份。导入默认预览，`--apply` 才写入，只允许导入空库。ID/token、公告游标、反馈、云控和管理员凭据保留，未知状态字段保留。

状态校验和 v1/v2 存档升级暂时通过常驻 Node 子进程复用原 TS 校验器，详见 [bridge/README.md](bridge/README.md)。写服务仍需要 Node 22+；启动时先检查校验子进程能否应答，失败时不开监听端口。只有读服务不启动该进程。原生 Rust 校验器覆盖全部可选字段并通过差分测试后才能移除此 fallback。

命令行 `serve` 默认禁止写入，`/health` 返回 `readOnly: true`、`websocketPush: false`。指定 `--enable-writes` 开放写入接口。反馈与管理写操作在只读模式返回 501。私网管理端口通过 `--admin-listen` 单独启用，只接受私有或回环 IPv4，允许网段默认是监听 IP 的 `/32`。

## 运行

```sh
cargo build --release --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# 输入必须是停机取得的一致快照，不要一边写入一边复制生产 data/。
./target/release/terrarium-server import-legacy --from /path/to/snapshot --database data/terrarium.sqlite
./target/release/terrarium-server import-legacy --from /path/to/snapshot --database data/terrarium.sqlite --apply
./target/release/terrarium-server check --database data/terrarium.sqlite
./target/release/terrarium-server serve --database data/terrarium.sqlite --listen 127.0.0.1:8790
# 仅在隔离数据副本上验证写入。
./target/release/terrarium-server serve --database data/terrarium.sqlite --listen 127.0.0.1:8790 --enable-writes
# 可选私网管理台；端口和允许网段按部署设置。
./target/release/terrarium-server serve --database data/terrarium.sqlite --listen 127.0.0.1:8790 --enable-writes --admin-listen 192.168.1.10:8791 --admin-cidr 192.168.1.0/24
# 凭据存在 SQLite 中。导入的管理员凭据直接延用；写入模式启用管理监听时才自动初始化缺失凭据。
./target/release/terrarium-server admin-token --database data/terrarium.sqlite
# 也可显式初始化；重复执行保留原值。
./target/release/terrarium-server admin-token --database data/terrarium.sqlite --create
./target/release/terrarium-server backup --database data/terrarium.sqlite --output backup.sqlite
./target/release/terrarium-server export-legacy --database data/terrarium.sqlite --output /path/to/new-export
```

导出和备份要求目标不存在。未更新的旧文档逐字保留；报告或注册更新后分别重新导出排行榜或凭据，其他文档仍保留原始内容。数据库和备份包含凭据，应与生产 `data/` 一样限制权限，不进入源码仓。SIGINT/SIGTERM 停止公共和管理 HTTP 服务，校验子进程随服务退出。

公网页面 Logo 由 `logo` 云控选择 0.8.6「闲看生机」或 0.8.7「新邻初见 · 第一阶段」，支持定时切换，打开的页面每分钟及恢复可见时同步。0.8.7 使用粗体 SVG 字标：MiSans Bold 字形转为轮廓，在笔画内融入新芽、瓢虫斑点和葵花金色。支持深浅主题、小屏幕和减少动态效果设置，无额外图片或字体请求。字形来自 Xiaomi MiSans 字体包；字体许可证保留在 `web/admin/assets/MiSans-LICENSE.pdf`。

`web/` 的公共页面、渲染 bundle 和 `web/admin/` 管理台来自生态箱 0.8.7 的 `server/` 与 `dist/console/`；代码按 MIT 许可证保留，字体与图标许可证位于 `web/admin/assets/`。管理台内存标签注明主进程范围。

箱子与凭据按行存储；公告、反馈、云控和协议通知保留兼容 JSON 文档，更新在 SQLite 写事务中完成。公告和反馈各有 2000 条上限。此阶段保持 schema 1，不增加磁盘格式迁移。

## 兼容验证

`cargo test --locked` 使用临时数据库、本地真实端口和合成状态，不访问外部服务。64 组 TS 积分样本、70 组规范 JSON hash 样本用于核对计分和校验和，另覆盖补丁路径、旧存档升级、校验进程退出或取消、鉴权、WS 重连、通知去重、计划生效、管理端口隔离、反馈容量与限流、并发通知游标、写入失败和导出备份。

现有 World 客户端的人工兼容检查：在装有 `tsx` 的 Cortico 检出中执行以下命令。脚本启动临时 Rust 服务、使用合成地/水箱检查完整/增量提交、排行榜、公告、重连、凭据复用、停止与导出，不接触部署数据。

```sh
node --import tsx /path/to/terrarium-server/scripts/check-client.mjs /path/to/Cortico/packages/cortico-world-terrarium
node --import tsx /path/to/terrarium-server/scripts/check-admin.mjs /path/to/Cortico/packages/cortico-world-terrarium
```

管理兼容脚本用旧 TS 管理 CLI 操作 Rust，再停止 Rust 并导出到临时目录；旧 TS 服务端在相同回环端口启动，检查状态、分数、凭据、公告、反馈与云控，并继续写入反馈和公告。所有记录均为合成数据。

## 生产部署与跨格式迁移

详见 [docs/deployment.md](docs/deployment.md)。Native 发布包使用 data epoch 2，旧 JSON 发布包使用 epoch 1。同 epoch 的更新沿用控制器；跨格式切换由迁移命令停写、备份、导入、检查导出后执行，回滚先导出最新 SQLite 数据。管理员凭据仍在库内，旧 `admin.mjs` 通过 `data/admin-token` 读取相同凭据。

```sh
node scripts/pack-release.mjs --id server-rust-example --revision <git-sha> --output /path/to/new-release
# 在目标主机安装，先保持当前版本服务。
node /path/to/new-release/release.mjs install --root /path/to/deployment --source /path/to/new-release
# 默认只预览；--apply 才停止用户服务并迁移。
node scripts/migrate-storage.mjs migrate --root /path/to/deployment --id server-rust-example
node scripts/migrate-storage.mjs migrate --root /path/to/deployment --id server-rust-example --apply
node scripts/migrate-storage.mjs rollback --root /path/to/deployment --apply
```

在目标主机上用临时用户服务演练迁移、带最新数据回滚、启动失败恢复和中断启动检查：

```sh
node scripts/check-migration.mjs /path/to/installed-ts-release /path/to/native-release
node scripts/check-snapshot.mjs /path/to/legacy/server.mjs /path/to/isolated-snapshot
```

后续将校验与旧存档升级移植到 Rust，移除 Node fallback，并持续测量实际提交延迟和查询负载。

当前使用一个 SQLite 数据库，保留表级边界。是否拆分数据库或增加缓存，由提交延迟、WAL 大小和查询负载的测量决定。

## 邻里花园与展示云控

`GET /api/forum/status` 返回开放状态。论坛默认关闭；`/forum` 公开页面只读，支持深色模式、搜索、分页、帖子与关注资料。其他论坛接口关闭时返回 HTTP 503、`code: forum_closed` 和“未开放论坛”，不创建论坛数据库。

- `GET /api/forum/threads`：`q` 搜索主题及回复；`author`、`followingOf` 筛选作者或关注动态。默认 5 条，最多 20 条，`before=nextCursor` 读取下一页。
- `GET /api/forum/threads/{id}`：原帖与倒序回复，采用相同分页参数。
- `GET /api/forum/profiles/{id}`：帖子数、关注数、粉丝数与分页关注名单，后续页传 `after=nextCursor`。
- `POST /api/forum/threads`：`title`、`body` 发帖；`POST /api/forum/threads/{id}/replies`：`body` 回复。
- `PUT /api/forum/likes`：`postId`、`liked`；`PUT /api/forum/follows`：`targetId`、`following`。相同目标值重复提交不增加计数。

写操作使用已注册箱子的 Bearer token，body 携带该箱子 `id` 与 `clientVersion`；不会创建新身份。线程标题 120 字、正文 4000 字，回复 2000 字；写请求最多 16 KiB。每箱新帖间隔 1 分钟、回复 10 秒、点赞/关注变更各 1 秒。每箱最多 500 个主题、5000 条回复、5000 个点赞、500 个关注；全站最多 100000 条帖子与回复。论坛同时处理最多 12 个请求，繁忙时返回 429。搜索使用 SQLite FTS5 trigram，短于三个字符时按字面子串搜索；正文作为文本展示。

私网管理台的 `PUT /api/admin/controls/forum` 接受 `enabled` 与可选 `at`，`PUT /api/admin/controls/logo` 接受 `version`（0.8.6 或 0.8.7）与可选 `at`；`{cancel:true}` 取消对应定时。立即设置会清除原定时。`GET /api/presentation` 返回所选字标。默认旧字标与关闭论坛，云控状态随主库备份和迁移。

论坛保存在主库同目录的 `terrarium.forum.sqlite`（主库文件名扩展为 `.forum.sqlite`），独立 schema 1，不迁移或修改生态箱 schema；生态箱 data epoch 仍为 2。主库与论坛分别备份：`terrarium-server backup-forum --database data/terrarium.sqlite --output forum-backup.sqlite` 使用 SQLite 一致快照，拒绝覆盖已有文件。恢复时停服，将论坛备份放回原 sidecar 路径。旧服务端回滚保留 sidecar，但不提供论坛接口；旧 JSON 导出不包含论坛，迁移机器时需单独携带论坛备份。不要运行旧版全目录恢复来覆盖较新的论坛数据。

排行榜摘要在 SQL 中选出排名页和自己的条目后逐条解析完整状态；不会同时解析所有箱子。Native 运行时使用 2 个异步工作线程、最多 8 个阻塞线程。内存应按 systemd 整组服务测量，包含 Node 校验子进程；论坛和报告队列上限用于约束并发占用，不代表任意连接量下的总内存保证。
