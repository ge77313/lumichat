$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { throw '未找到 Docker，请先安装 Docker Desktop。' }
docker compose version | Out-Null
Write-Host '[1/3] 构建并启动 LumiChat...'
docker compose up --build -d
Write-Host '[2/3] 等待服务健康检查...'
$healthy = $false
1..30 | ForEach-Object {
  try { Invoke-RestMethod 'http://127.0.0.1:8080/api/health' -TimeoutSec 2 | Out-Null; $healthy = $true; return } catch { Start-Sleep -Seconds 1 }
}
if (-not $healthy) { throw '服务未通过健康检查，请运行 docker compose logs --tail=100' }
Write-Host '[3/3] 部署完成：http://localhost:8080'
