# Puts topbar, reshell, rust-explorer and rs-shell into %LOCALAPPDATA%\plugin-launcher\bin (a folder you own: updates never need
# admin) and starts them at sign-in from your Run key as an OVERLAY on top of explorer.exe, which stays the real shell.
# (Replacing the shell itself stopped working: Windows no longer started our rs-shell at logon.) Run from a NORMAL PowerShell:
#   powershell -ExecutionPolicy Bypass -File scripts\user-mode.ps1            install / refresh
#   powershell -ExecutionPolicy Bypass -File scripts\user-mode.ps1 -Undo      remove the Run entry and the shell override
# The old elevated "topbar" logon task would start a second bar: it is deleted once with
#   schtasks /delete /tn topbar /f      (administrator PowerShell; the script prints this if the task still exists)
param([switch]$Undo)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$bin = "$env:LOCALAPPDATA\plugin-launcher\bin"
$winlogon = 'HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon'
$run = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Run'

# our own Shell override (if any) is never wanted any more
cmd /c "reg delete `"$winlogon`" /v Shell /f >nul 2>&1"

$folderKeys = 'HKCU:\Software\Classes\Directory\shell\open', 'HKCU:\Software\Classes\Folder\shell\open', 'HKCU:\Software\Classes\Drive\shell\open'

if ($Undo) {
    cmd /c "reg delete `"$run`" /v rs-overlay /f >nul 2>&1"
    foreach ($k in $folderKeys) { Remove-Item $k -Recurse -ErrorAction SilentlyContinue }   # only our HKCU overrides; the system's verbs stay
    'overlay: removed from sign-in, folders open in Explorer again (files in ' + $bin + ' left alone)'
    return
}

$src = @{
    'rs-shell.exe'      = "$root\target\release\rs-shell.exe"
    'rust-explorer.exe' = "$root\target\release\rust-explorer.exe"
    'topbar.exe'        = "$root\target\release\topbar.exe"
    'reshell.exe'       = "$root\target\release\reshell.exe"
}
foreach ($k in $src.Keys) { if (-not (Test-Path $src[$k])) { throw "missing $($src[$k]): build it first (scripts\build-plugins.ps1)" } }
New-Item -ItemType Directory $bin -Force | Out-Null
foreach ($k in $src.Keys) {
    $to = "$bin\$k"
    # a running exe cannot be overwritten but can be renamed
    if (Test-Path $to) { Rename-Item $to ("$k." + (Get-Date -Format HHmmssfff) + '.old') -Force }
    Copy-Item $src[$k] $to -Force
}
Get-ChildItem $bin -Filter *.old -ErrorAction SilentlyContinue | ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
reg add $run /v rs-overlay /t REG_SZ /d "`"$bin\rs-shell.exe`" --overlay" /f | Out-Null
"overlay: starts at sign-in from $bin"
# Folders and drives opened from other programs / the desktop / Win+R open in rust-explorer. DelegateExecute is emptied on
# purpose: while it is set, Windows ignores the command and uses Explorer's built-in handler.
foreach ($k in $folderKeys) {
    New-Item "$k\command" -Force | Out-Null
    Set-ItemProperty "$k\command" -Name '(default)' -Value "`"$bin\rust-explorer.exe`" `"%1`""
    Set-ItemProperty "$k\command" -Name 'DelegateExecute' -Value ''
}
'folders: open in rust-explorer (undo: scripts\user-mode.ps1 -Undo)'
cmd /c "schtasks /query /tn topbar >nul 2>&1"   # through cmd: schtasks' stderr would be a terminating error under $ErrorActionPreference = Stop
if ($LASTEXITCODE -eq 0) {
    ''
    'ONE admin step (administrator PowerShell, once):   schtasks /delete /tn topbar /f'
}
''
"done. Update later with: scripts\dev-update.ps1 <topbar|rs-shell|rust-explorer|reshell>   (no admin)"
