# Full build: every plugin into payload\, then the launcher with all of them embedded (one self-contained exe).
#   powershell -ExecutionPolicy Bypass -File scripts\build-all.ps1
# Close a running plugin-launcher.exe first.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
& "$PSScriptRoot\build-plugins.ps1"
Push-Location $root
try { cargo build --release -p plugin-launcher; if ($LASTEXITCODE) { throw 'launcher build failed' } } finally { Pop-Location }
Write-Host "done: $root\target\release\plugin-launcher.exe"
