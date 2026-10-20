<!-- Owner: src/main.rs, src/store.rs, src/http.rs, src/scoring.rs -->
# terrarium-server

生态箱独立服务端，Rust + SQLite 迁移的第一阶段。生产环境仍使用 `cortico-world-terrarium` 的 TypeScript 服务端；本仓库目前是只读预览，不能替换生产入口。

已实现：

- Axum HTTP 服务：排行榜、排行榜摘要、分页观察页、单箱状态与按需回放读取。
- SQLite WAL，单个持久写连接，独立读连接；写事务使用 `synchronous=FULL`。每箱一行，通过 revision 比较后更新；数据库操作在阻塞线程执行。
- 旧 JSON 导入、导出、数据库完整性检查、在线备份。导入默认预览，只有 `--apply` 才写入；只允许导入空库。
- ID、token、公告游标、反馈、云控和管理员凭据保留。未知 JSON 字段保留。64 组 TypeScript 积分样本用于校验 Rust 积分计算。

未实现：注册、HTTP/WS 提交、增量补丁及校验、通知投递、管理写入和生产存储迁移。相应接口返回错误，不会报告提交成功。`/health` 明确返回 `readOnly: true` 与 `websocketPush: false`。预览只允许监听回环地址。

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
./target/release/terrarium-server backup --database data/terrarium.sqlite --output backup.sqlite
./target/release/terrarium-server export-legacy --database data/terrarium.sqlite --output /path/to/new-export
```

导出和备份要求目标不存在。未更新的旧文档逐字保留；报告更新后重新导出排行榜，其他文档仍保留原始内容。数据库和备份包含凭据，应与生产 `data/` 一样限制权限。数据文件不进入源码仓。

`web/` 是生态箱 0.8.6 的公共页面与渲染 bundle，按 MIT 许可证保留。当前公告与反馈接口尚未迁移，预览不会显示公告或接收反馈。

## 后续迁移

1. 移植注册、状态校验、WS 全量与增量、规范 JSON 校验和、版本云控及协议通知，使用旧客户端做差分测试。
2. 移植管理接口、公告、反馈和计划任务，补齐 Native 发布包与停止流程。
3. 在一致的生产快照上校验箱子、凭据、排行榜、通知游标及导出结果，再进行影子读取和负载验证。
4. 安排停机迁移。SQLite 与旧 JSON 的写入格式不同，使用新的 data epoch；迁移完成前不允许跨格式自动回滚。回滚前须停写并导出最新数据，不能仅切换旧二进制。

当前先用一个 SQLite 数据库，保留表级边界。是否拆分数据库、缓存或改用其他存储，由提交延迟、WAL 大小和查询负载的测量决定。SQLite WAL 允许读取与写入并行，但同一时刻只有一个写者；参见 [SQLite WAL](https://www.sqlite.org/wal.html)。
