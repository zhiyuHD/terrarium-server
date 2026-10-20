<!-- Owner: src/main.rs, src/store.rs, src/http.rs, src/scoring.rs, src/ingestion.rs, src/push.rs, src/protocol.rs, src/policy.rs, src/legacy_validation.rs, scripts/check-client.mjs -->
# terrarium-server

生态箱独立服务端，使用 Rust + SQLite 逐步迁移。生产环境仍运行 `cortico-world-terrarium` 的 TypeScript 服务端；本仓库是回环地址上的迁移预览，管理与反馈接口尚未迁移，不能替换生产入口。

已实现：

- 排行榜、摘要、分页观察页、单箱状态与按需回放读取。
- 自动分配 ID/token，兼容旧 ID 注册，Bearer 鉴权，HTTP 完整上报和 WS 完整/增量上报。提交上限 8 MiB，每连接最多 8 个待处理报告，WS 每 30 秒检查心跳。
- 与 TS 一致的规范 JSON SHA-256、基线 revision/hash 和目标 hash 校验。首次连接、重连及校验失败要求完整状态；成功确认在 SQLite 事务提交后发送。
- 最低客户端版本、HTTP Push 开关及定时生效规则，忽略提交时返回 `accepted: false`。协议升级提醒使用独立记录，保留既有通知 ID，不进入管理员公告列表；支持公开公告及鉴权公告轮询。
- SQLite WAL，持久写连接、独立读连接、`synchronous=FULL`。每箱一行，比较 revision 后更新；运行时数据库操作在阻塞线程执行。
- 旧 JSON 导入、导出、完整性检查、在线备份。导入默认预览，`--apply` 才写入，只允许导入空库。ID/token、公告游标、反馈、云控和管理员凭据保留，未知状态字段保留。

状态校验和 v1/v2 存档升级暂时通过常驻 Node 子进程复用原 TS 校验器，详见 [bridge/README.md](bridge/README.md)。写入预览仍需要 Node 22+；只有读服务不启动该进程。原生 Rust 校验器覆盖全部可选字段并通过差分测试后才能移除此 fallback。

默认 `serve` 禁止写入，`/health` 返回 `readOnly: true`、`websocketPush: false`。显式指定 `--enable-writes` 才开放已迁移的写入接口。管理与反馈接口返回 501，不会报告成功。预览只允许监听回环地址。

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
./target/release/terrarium-server backup --database data/terrarium.sqlite --output backup.sqlite
./target/release/terrarium-server export-legacy --database data/terrarium.sqlite --output /path/to/new-export
```

导出和备份要求目标不存在。未更新的旧文档逐字保留；报告或注册更新后分别重新导出排行榜或凭据，其他文档仍保留原始内容。数据库和备份包含凭据，应与生产 `data/` 一样限制权限，不进入源码仓。SIGINT/SIGTERM 停止 HTTP 服务，校验子进程随服务退出。

`web/` 是生态箱 0.8.6 的公共页面与渲染 bundle，按 MIT 许可证保留。公告可以读取；反馈提交与管理操作尚不可用。

## 兼容验证

`cargo test --locked` 使用临时数据库、本地真实端口和合成状态，不访问外部服务。64 组 TS 积分样本、70 组规范 JSON hash 样本用于核对计分和校验和，另覆盖补丁路径、旧存档升级、校验进程退出或取消、鉴权、WS 重连、通知去重、计划生效、写入失败和导出备份。

现有 World 客户端的人工兼容检查：在装有 `tsx` 的 Cortico 检出中执行以下命令。脚本启动临时 Rust 服务、使用合成地/水箱检查完整/增量提交、排行榜、公告、重连、凭据复用、停止与导出，不接触部署数据。

```sh
node --import tsx /path/to/terrarium-server/scripts/check-client.mjs /path/to/Cortico/packages/cortico-world-terrarium
```

## 后续迁移

1. 移植管理接口、公告与反馈写入、运行指标，补齐 Native 发布包；将校验与旧存档升级移植到 Rust，移除 Node fallback。
2. 在一致的生产快照上校验箱子、凭据、排行榜、通知游标及导出结果，再进行影子读取和负载验证。
3. 安排停机迁移。SQLite 与旧 JSON 写入格式不同，使用新的 data epoch；迁移完成前不允许跨格式自动回滚。回滚前须停写并导出最新数据，不能仅切换旧二进制。

当前使用一个 SQLite 数据库，保留表级边界。是否拆分数据库或增加缓存，由提交延迟、WAL 大小和查询负载的测量决定。
