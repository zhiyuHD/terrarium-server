<!-- Owner: ../src/photos.rs, ../src/photo_cache.rs, ../src/photo_notices.rs, ../src/http.rs, ../src/policy.rs, ../src/forum_bridge.rs, ../src/store.rs -->
# 临时照片链接

`POST /api/photos` 保留已有鉴权、状态 hash 校验与 PNG 响应体。成功后额外返回：

- `X-Terrarium-Photo-Url`：无需 Bearer token 的 HTTPS 图片地址。
- `X-Terrarium-Photo-Expires-At`：ISO 8601 到期时间，自图片生成完成起 24 小时。

`GET /api/photos/{到期毫秒数}-{64位随机十六进制}.png` 返回相同 PNG。URL 是持有即可访问的临时凭证，地址中不含箱子 ID、token 或配文。未知、非法和过期地址返回 404；下载回执使用 `Cache-Control: no-store`，避免 CDN 把图片保留到到期以后。照片固定于拍摄瞬间，后续箱子变化不影响图片。仅持有链接的人可取图，不提供相册或图片列表。

`CORTICO_TERRARIUM_PUBLIC_URL` 指定对外 HTTPS 源站，默认 `https://cortico-box.zhiyuhub.top`。只接受 HTTPS origin，可含端口，不接受路径、查询参数或 URL 内的用户名密码。地址由服务端配置生成，不信任客户端的 Host 或转发头。主站与镜像共用相同缓存，生产使用主站作为默认照片地址。

现有 World 不读取新响应头。兼容回执通过公告轮询通道定向投递，标题为“拍照链接回执”，正文 `[photo-result]` 后是 JSON：`boxId`、`habitat`、`capturedAt`、`stateHash`、`caption`、`url`、`expiresAt`、`purpose`。回执只属于此次拍照，不要求主动联系主人；模型可在已有对话需要发送图片、工具只接受 HTTPS 地址时使用 `url`。轮询游标确认后回执从待投递队列移除；未确认的回执随链接到期清除。它不进入公开公告、管理公告列表或反馈箱。无客户端升级依赖，回执将在客户端下一次正常公告轮询时到达。

PNG 保存在数据库同级的 `terrarium.photos/`（数据库名不同时沿用其文件名主干），位于原子发布目录之外。服务端重启、更新和回滚不延长有效期；支持本功能的版本重启后仍能读取未到期文件。缓存是可丢弃的临时派生数据，不进入 SQLite schema、数据 epoch、旧格式导出或数据库备份。旧二进制回滚后不提供取图路由，再升级时按原到期时间清理。

缓存最多 256 MiB、4096 张图；同一箱子最多保留 32 条未确认拍照回执。容量耗尽时新拍照返回 503，不提前删除其他尚未到期的链接。写入失败或回执入队失败时不发布 URL，并删除此次临时图片。文件权限为 0600，目录为 0700。每次写入前清理过期文件；启动时及每 60 秒后台清理，GET 也会立即拒绝并删除已过期文件。下载最多同时进行 16 路，超出返回 429；每路使用 64 KiB 缓冲，进程不保留完整相册在内存中。系统仍可能使用可回收的文件页缓存。

验证：`cargo test --locked` 覆盖真实 PNG 回执与下载一致、快照变化后照片不变、24 小时边界、缓存重开、无读取时清理、容量上限、非法路径、私有收件人、公告列表隔离、游标确认与导出恢复。`cargo clippy --all-targets --locked -- -D warnings` 和 `cargo fmt --check` 验证构建规范。
