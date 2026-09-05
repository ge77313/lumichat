# LumiChat

LumiChat 是一个从零实现、MIT 许可的超轻量自托管聊天系统。项目采用单体 Rust 后端、零构建依赖的静态 Web 前端、SQLite 和本地文件存储，保持轻量、独立和可自托管。

## 核心能力

- 注册、登录与管理员账号管理（首个注册用户自动成为管理员）
- 频道、好友私聊、历史消息、回复、编辑和删除
- WebSocket 实时消息与一对一 WebRTC 语音/视频通话
- 图片/文件上传、聊天图片排版与消息搜索
- 管理员分页查看全站聊天记录和集中图片集，并可一键清空全部聊天
- Apple 风格的桌面、平板和手机响应式界面
- SQLite WAL 持久化；不需要 Redis、Kafka、RabbitMQ 或 Elasticsearch

## 隐私好友制

普通用户看不到全站成员目录，也不能通过模糊搜索、分页、资料接口或 WebSocket 在线列表枚举用户。联系人必须通过精确用户名、精确 UID、随机邀请链接或二维码主动添加，并在对方接受后建立好友关系。

- 非好友不能发私聊、发起语音/视频呼叫或查看详细资料。
- A 与 B、B 与 C 成为好友，不会让 A 看到 C。
- 删除好友后保留历史记录，但双方不能继续发送新消息。
- 拉黑会同时解除好友关系，并阻止再次申请、私聊和呼叫。
- 邀请 token 为随机 48 字符串，可重新生成使旧链接立即失效。
- 旧版本已有私聊记录的双方仅在首次升级迁移时自动成为联系人。
- 管理员保留用户管理和全站记录审计权限；普通用户访问管理接口统一返回 404。

## 项目结构

```text
lumichat/
├── server/main.rs                   # Axum API、权限、WebSocket、SQLite schema
├── web/                             # 零依赖 HTML / CSS / JavaScript 前端
├── tests/friend_regression.py       # 好友制和防枚举回归
├── tests/ws_signaling_regression.py # 通话信令权限回归
├── Dockerfile                       # 两阶段精简镜像
├── docker-compose.yml               # 单服务与持久数据卷
├── install.sh / install.ps1         # Linux / Windows 一键启动脚本
├── DEPLOY.md                         # 一键部署与 HTTPS 说明
├── BASELINE.md                      # 性能基线
├── IMPLEMENTATION_REPORT.md         # 本次好友制实现报告
└── design-qa.md                     # UI 对照与响应式验收
```

## Docker 部署

```bash
docker compose up --build -d
```

### 一键部署

在已安装 Docker 的 Linux 服务器上，先拉取私有仓库，再运行安装脚本：

```bash
git clone https://github.com/ge77313/lumichat.git
cd lumichat
chmod +x install.sh
./install.sh
```

也可以在已有项目目录中直接执行：

```bash
bash install.sh
```

Windows Docker Desktop 用户运行：

```powershell
./install.ps1
```

本仓库为 Public，可直接通过 GitHub 拉取源码。脚本会构建镜像、启动容器并等待健康检查，不会删除已有数据。

更多部署选项、反向代理与 HTTPS 配置见 [DEPLOY.md](DEPLOY.md)。

默认监听 `http://localhost:8080`，数据位于命名卷 `lumichat-data`。生产环境应在服务前配置 HTTPS 反向代理；麦克风和摄像头在非 localhost 环境下需要 HTTPS。

可用环境变量：

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `LUMICHAT_BIND` | `0.0.0.0:8080` | 监听地址 |
| `LUMICHAT_DATABASE` | `data/lumichat.db` | SQLite 文件 |
| `LUMICHAT_UPLOADS` | `data/uploads` | 上传目录 |
| `LUMICHAT_WEB` | `web` | 静态前端目录 |
| `LUMICHAT_PUBLIC_URL` | 请求来源 | 邀请链接的公开站点地址 |
| `RUST_LOG` | `lumichat=info,tower_http=info` | 日志级别 |

当前 `docker-compose.yml` 将容器内存上限设为 128 MiB。浏览器通过免费 STUN 尝试建立点对点媒体连接；严格 NAT 或企业防火墙场景应额外配置 TURN。

## 好友与权限 API

除注册、登录和健康检查外，请求都使用 `Authorization: Bearer <token>`。

| 方法与路径 | 功能 |
| --- | --- |
| `GET /api/friends` | 当前用户联系人列表 |
| `POST /api/friends/lookup` | 精确用户名、UID 或邀请 token 查询 |
| `GET/POST /api/friend-requests` | 查看或发送好友申请 |
| `POST /api/friend-requests/:id/accept` | 接受申请 |
| `POST /api/friend-requests/:id/reject` | 拒绝申请 |
| `POST /api/friend-requests/:id/cancel` | 取消发出的申请 |
| `DELETE /api/friends/:id` | 删除好友并保留历史消息 |
| `POST /api/friends/:id` | 拉黑用户 |
| `POST /api/friends/:id/unblock` | 解除拉黑 |
| `GET /api/friend-invite` | 获取个人邀请链接和二维码 |
| `POST /api/friend-invite/regenerate` | 使旧 token 失效并重新生成 |
| `GET /api/users/:id` | 查看自己、好友或管理员可见资料 |
| `GET /api/users` | 管理员专用全站用户列表 |
| `GET/DELETE /api/admin/messages` | 管理员分页记录/图片集与清空全部聊天 |

好友精确查询限制为每用户每分钟 10 次、每小时 40 次。未命中、自查、拉黑或未授权资料访问使用统一的未找到响应，减少账号存在性泄露。

## 本地开发与验证

需要 Rust 1.85 或更高版本。

```bash
cargo run
cargo fmt --check
node --check web/app.js
python tests/friend_regression.py http://127.0.0.1:18084
python tests/ws_signaling_regression.py 18084
```

完整测试矩阵、数据库变更和未自动化项目见 [IMPLEMENTATION_REPORT.md](IMPLEMENTATION_REPORT.md)，性能数据见 [BASELINE.md](BASELINE.md)。

## 安全说明

- 登录 token 当前长期有效；生产强化仍建议加入过期、撤销和审计日志。
- 上传目前限制为 10 MiB；公开环境应进一步加入 MIME 白名单与病毒扫描。
- 管理员的“清空全部聊天”会删除数据库消息及关联本地附件，操作不可撤销。
- 服务返回 `X-Robots-Tag: noindex, nofollow, noarchive, nosnippet, noimageindex`，并提供禁止抓取的 `robots.txt`；这能表达不收录意愿，但不能替代访问控制。
