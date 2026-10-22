$ErrorActionPreference = 'Stop'
$privateRoot = Join-Path $env:USERPROFILE '.codex\local-secrets\saas-stores'
$runtimePath = Join-Path $privateRoot 'runtime.json'
$runtime = Get-Content -LiteralPath $runtimePath -Raw | ConvertFrom-Json
foreach ($required in @('STORES_DATABASE_URL', 'PLATFORM_API_ORIGIN', 'PLATFORM_CLIENT_ID', 'PLATFORM_CLIENT_SECRET')) {
    if (-not $runtime.$required) { throw "Missing private runtime setting: $required" }
}
$envPath = Join-Path $privateRoot 'docker.env'
$lines = foreach ($property in $runtime.PSObject.Properties) {
    $value = [string]$property.Value
    if ($value.Contains("`n") -or $value.Contains("`r")) { throw 'Runtime values must be single-line' }
    '{0}={1}' -f $property.Name, $value
}
[IO.File]::WriteAllText($envPath, ($lines -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
$linuxEnvPath = (& wsl --exec wslpath -a $envPath).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve WSL environment path' }
$existing = & wsl --exec docker ps -a --filter 'name=^/codex-stores-api-local$' --format '{{.Names}}'
if ($existing) {
    Write-Host 'Starting the existing local container; keep this terminal open to preserve the WSL session.'
    & wsl --exec docker start -a codex-stores-api-local
    exit $LASTEXITCODE
}
& wsl --exec docker run -d --name codex-stores-api-local --network host --read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m --cap-drop ALL --security-opt no-new-privileges --env-file $linuxEnvPath codex-stores-api:local
if ($LASTEXITCODE -ne 0) { throw 'Stores API container failed to start' }
Write-Host 'Stores API container created. Verify /stores-api/health/ready before opening the application.'
Write-Host 'Keep this terminal open. Stop explicitly with: wsl --exec docker stop codex-stores-api-local'
& wsl --exec docker attach --sig-proxy=false codex-stores-api-local
