# Rebuild one piece and swap it into %LOCALAPPDATA%\plugin-launcher\bin (set up by user-mode.ps1). No admin, no UAC.
#   powershell -ExecutionPolicy Bypass -File scripts\dev-update.ps1 <topbar|rs-shell|rust-explorer|reshell>
# topbar / reshell are restarted by rs-shell within ~6 s; rs-shell and rust-explorer take effect at the next sign-in
# (or restart rust-explorer --desktop yourself). Never kills rs-shell: it is the session's shell.
param([Parameter(Mandatory)][ValidateSet('topbar', 'rs-shell', 'rust-explorer', 'reshell')][string]$Name)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$bin = "$env:LOCALAPPDATA\plugin-launcher\bin"
$exe = "$Name.exe"
if (-not (Test-Path "$bin\rs-shell.exe")) { throw "run scripts\user-mode.ps1 first" }
Push-Location $root; cargo build --release -p $Name; $rc = $LASTEXITCODE; Pop-Location
if ($rc -ne 0) { throw "cargo build failed" }
$to = "$bin\$exe"
if (Test-Path $to) { Rename-Item $to ("$exe." + (Get-Date -Format HHmmssfff) + '.old') -Force }
Copy-Item "$root\target\release\$exe" $to -Force
Get-ChildItem $bin -Filter "$exe.*.old" | ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
switch ($Name) {
    'topbar'  { Stop-Process -Name topbar -Force -ErrorAction SilentlyContinue; "topbar stopped: rs-shell starts the new one in a few seconds" }
    'reshell' { Stop-Process -Name reshell -Force -ErrorAction SilentlyContinue; Start-Process $to -WindowStyle Hidden; "reshell restarted" }
    default   { "$exe replaced: takes effect at the next sign-in" }
}
