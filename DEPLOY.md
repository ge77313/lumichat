# LumiChat 一键 Docker 部署

## Linux 服务器

把项目目录上传到服务器后执行：

```bash
cd lumichat
chmod +x install.sh
./install.sh
```

脚本会检查 Docker/Compose、构建镜像、启动容器并轮询 `/api/health`。它不会删除数据库、上传文件或执行 `down -v`。数据保存在 `lumichat-data` 命名卷中。

后续更新：

```bash
git pull
./install.sh
```

常用命令：

```bash
docker compose ps
docker compose logs -f lumichat
docker compose restart
```

## Windows / Docker Desktop

在 PowerShell 中运行：

```powershell
.\install.ps1
```

然后打开 `http://localhost:8080`。

## 绑定域名和 HTTPS

一键脚本只负责应用容器。生产环境建议让 Nginx、Caddy 或 Cloudflare Tunnel 终止 HTTPS，再把流量转发到 `127.0.0.1:8080`，并在 `docker-compose.yml` 中设置 `LUMICHAT_PUBLIC_URL`。

浏览器只有在 HTTPS 页面下才会允许远程摄像头和麦克风。域名解析、证书和 TURN 中继属于网络层配置，不需要引入 Redis 等服务。

## 发布成真正的一条命令

将项目放到 GitHub 后，可以执行：

```bash
curl -fsSL https://raw.githubusercontent.com/你的账号/lumichat/main/install.sh | bash
```

更安全的做法是先下载并检查脚本，再执行；不要从未知来源直接执行远程脚本。
