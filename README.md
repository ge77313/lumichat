# LumiChat

LumiChat 是一个从零实现的、MIT 许可的超轻量自托管聊天原型。它借鉴轻量团队聊天产品的核心体验，但不包含、复制或规避 VoceChat 的代码、素材与授权机制。

## 它有什么

- 用户注册与登录（Argon2 密码哈希；第一个注册用户自动成为管理员）
- 公开频道、私聊、历史消息
- WebSocket 实时消息
- 基础用户管理（管理员可启用或停用账号）
- 最大 10 MiB 的本地文件上传
- 简单消息搜索
- 一对一 WebRTC 语音与视频通话（媒体点对点传输）
- 深色模式与移动端布局
- SQLite WAL 持久化；不需要 Redis、消息队列或搜索服务

## 项目结构

```text
lumichat/
├── server/main.rs       # Axum API、WebSocket、SQLite schema 与静态文件服务
├── web/                 # 零依赖 HTML / CSS / JavaScript 前端
├── Dockerfile           # 两阶段精简镜像
├── docker-compose.yml   # 单服务部署与持久卷
├── dist/                # 本次验证生成的 Windows x86_64 二进制
├── Cargo.toml
├── Cargo.lock
├── BASELINE.md          # 当前机器上的性能基线
└── README.md
```

## Docker 启动

```bash
docker compose up --build -d
```

打开 `http://localhost:8080`。首次注册的账号拥有管理员权限。数据保存在命名卷 `lumichat-data` 中。

> 生产环境请在 LumiChat 前面配置 HTTPS 反向代理。登录令牌目前长期有效，原型阶段未实现过期、找回密码、审计日志、病毒扫描和细粒度频道权限。

> 除 `localhost` 外，浏览器只允许 HTTPS 页面访问麦克风和摄像头。内置免费 STUN 用于发现点对点路径；严格 NAT 或企业防火墙环境若无法直连，需要另配 TURN 中继。

## 本地开发

需要 Rust 1.85 或更高版本。

```bash
cargo run
```

默认地址是 `http://localhost:8080`，数据库与上传文件位于 `data/`。

本次验证生成的 Windows 版本也可直接运行：

```powershell
.\dist\lumichat-windows-x86_64.exe
```

可用环境变量：

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `LUMICHAT_BIND` | `0.0.0.0:8080` | 监听地址 |
| `LUMICHAT_DATABASE` | `data/lumichat.db` | SQLite 文件 |
| `LUMICHAT_UPLOADS` | `data/uploads` | 上传目录 |
| `LUMICHAT_WEB` | `web` | 静态前端目录 |
| `RUST_LOG` | `lumichat=info,tower_http=info` | 日志级别 |

## API 摘要

除注册、登录和健康检查外，请求都使用 `Authorization: Bearer <token>`。

| 方法与路径 | 功能 |
| --- | --- |
| `POST /api/register`、`POST /api/login` | 注册、登录 |
| `GET /api/channels`、`POST /api/channels` | 频道列表、新建频道 |
| `GET/POST /api/channels/:id/messages` | 频道历史与发消息 |
| `GET/POST /api/dm/:user_id/messages` | 私聊历史与发消息 |
| `GET /api/ws?token=...` | 实时事件流 |
| `POST /api/upload` | 本地文件上传 |
| `GET /api/search?q=...` | 搜索可见消息 |
| `GET /api/users`、`PATCH /api/users/:id` | 用户列表与管理 |

## 轻量设计取舍

- 单进程同时提供 API、WebSocket 和静态资源。
- SQLite 使用 WAL、`synchronous=NORMAL` 和与实际查询对应的少量索引。
- 前端没有构建步骤或 npm 依赖。
- 进程内广播适合单实例；未来水平扩容时才需要外部消息总线。
- 搜索使用 SQLite `LIKE`，数据量变大后可迁移到 SQLite FTS5，仍无需 Elasticsearch。

## 验证

本项目的基本验证覆盖：健康检查、注册、默认频道、第二用户、频道消息、私信、搜索、上传、管理员停用用户和 WebSocket 握手。实测结果与环境记录在 [BASELINE.md](BASELINE.md)。

## 下一步建议

1. 加入会话过期、速率限制与 CSRF/来源校验。
2. 为上传添加 MIME 白名单、文件名下载头与病毒扫描钩子。
3. 添加频道成员与权限模型。
4. 用 FTS5 替换大数据量下的 `LIKE` 搜索。
5. 加入数据库备份、恢复与迁移版本管理。
