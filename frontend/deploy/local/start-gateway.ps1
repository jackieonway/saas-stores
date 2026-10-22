param(
    [string]$ReleaseRoot = (Join-Path $PSScriptRoot '..\..\..\artifacts\micro-apps'),
    [string]$Image = 'nginxinc/nginx-unprivileged:1.28.0-alpine'
)
$ErrorActionPreference = 'Stop'
$configPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'nginx.conf')).Path
$releasePath = (Resolve-Path -LiteralPath $ReleaseRoot).Path
if (-not (Test-Path -LiteralPath (Join-Path $releasePath 'stores\versions\1.0.1\index.html'))) {
    throw 'Publish the complete stores 1.0.1 release before starting the gateway.'
}
$linuxConfig = (& wsl --exec wslpath -a $configPath).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve nginx.conf in WSL' }
$linuxReleases = (& wsl --exec wslpath -a $releasePath).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve release directory in WSL' }
$existing = & wsl --exec docker ps -a --filter 'name=^/codex-stores-gateway-local$' --format '{{.Names}}'
if ($LASTEXITCODE -ne 0) { throw 'Cannot query Docker' }
if ($existing) { throw 'Local gateway already exists; inspect it before changing its configuration.' }
$dockerArgs = @('run', '--name', 'codex-stores-gateway-local', '--network', 'host',
    '--read-only', '--tmpfs', '/tmp:rw,noexec,nosuid,size=32m', '--cap-drop', 'ALL',
    '--security-opt', 'no-new-privileges', '--mount', "type=bind,src=$linuxConfig,dst=/etc/nginx/nginx.conf,readonly",
    '--mount', "type=bind,src=$linuxReleases,dst=/srv/micro-apps,readonly", $Image,
    'nginx', '-g', 'daemon off;')
Write-Host 'Local gateway: http://127.0.0.1:3000 -> platform :3002, stores API :3031'
Write-Host 'Keep the terminal open. Stop with: wsl --exec docker stop codex-stores-gateway-local'
& wsl --exec docker @dockerArgs
if ($LASTEXITCODE -ne 0) { throw 'Local gateway failed' }
