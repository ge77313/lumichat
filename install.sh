#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
cd "$ROOT_DIR"

if ! command -v docker >/dev/null 2>&1; then
  echo "错误：未找到 Docker，请先安装 Docker Engine。" >&2
  exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
  echo "错误：未找到 Docker Compose 插件，请安装 docker compose v2。" >&2
  exit 1
fi
if [ ! -f docker-compose.yml ]; then
  echo "错误：请在 LumiChat 项目目录运行此脚本。" >&2
  exit 1
fi

echo "[1/3] 构建并启动 LumiChat..."
docker compose up --build -d
echo "[2/3] 等待服务健康检查..."
for _ in $(seq 1 30); do
  curl -fsS http://127.0.0.1:8080/api/health >/dev/null 2>&1 && break
  sleep 1
done
if ! curl -fsS http://127.0.0.1:8080/api/health >/dev/null 2>&1; then
  echo "错误：服务未通过健康检查，查看日志：docker compose logs --tail=100" >&2
  exit 1
fi
echo "[3/3] 部署完成"
echo "访问地址：http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo '服务器IP'):8080"
echo "查看状态：docker compose ps"
echo "查看日志：docker compose logs -f lumichat"
