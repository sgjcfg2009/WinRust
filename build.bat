@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -Command "$c = (Get-Content -LiteralPath '%~f0' -Raw) -replace '(?s)^.*?REM === PS_START ===\r?\n',''; & ([scriptblock]::Create($c)) '%~dp0'"
if errorlevel 1 (
    echo.
    echo [ERROR] Build failed! Press any key to close...
    pause >nul
    exit /b 1
)
echo.
echo Press any key to close...
pause >nul
exit /b 0
REM === PS_START ===
try {
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Root = $args[0].TrimEnd('\')
$ToolsDir = Join-Path $Root 'tools'
$OutDir = Join-Path $Root 'out'
$BuildTmp = Join-Path $ToolsDir 'tmp'
$Jobs = [Math]::Max(1, [Environment]::ProcessorCount)
$env:CARGO_BUILD_JOBS = "$Jobs"

New-Item -ItemType Directory -Force -Path $ToolsDir, $OutDir, $BuildTmp | Out-Null
$env:TEMP = $BuildTmp
$env:TMP = $BuildTmp

function Download-MultiThread {
    param(
        [Parameter(Mandatory)][string]$Url,
        [Parameter(Mandatory)][string]$DestPath,
        [int]$Threads = 8
    )
    Write-Host "[Download] $Url -> $DestPath"
    $curl = Join-Path $env:SystemRoot 'System32\curl.exe'
    if (Test-Path $curl) {
        & $curl -fSL --retry 3 --connect-timeout 15 -o $DestPath $Url
        if ($LASTEXITCODE -eq 0 -and (Test-Path $DestPath) -and (Get-Item $DestPath).Length -gt 0) {
            return
        }
    }
    [Net.ServicePointManager]::DefaultConnectionLimit = 16
    $wc = New-Object System.Net.WebClient
    $wc.Headers.Add('User-Agent', 'WinRust-Builder/1.0')
    $wc.DownloadFile($Url, $DestPath)
}

function Find-MSVC {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vcPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
        if ($vcPath) { return $true }
    }
    if (Get-Command link.exe -ErrorAction SilentlyContinue) { return $true }
    return $false
}

function Ensure-MinGW {
    $mingwBin = Join-Path $ToolsDir 'mingw64\bin'
    if (Test-Path (Join-Path $mingwBin 'gcc.exe')) {
        $env:PATH = "$mingwBin;$env:PATH"
        return
    }
    Write-Host "[Tools] MSVC not found. Downloading portable MinGW-w64 into $ToolsDir\mingw64..."
    $headers = @{ 'User-Agent' = 'WinRust-Builder/1.0' }
    $rel = Invoke-RestMethod -Uri 'https://api.github.com/repos/brechtsanders/winlibs_mingw/releases/latest' -Headers $headers
    $asset = $rel.assets | Where-Object { $_.name -match 'winlibs-x86_64-posix-seh-gcc-.*-mingw-w64ucrt-.*\.zip$' } | Select-Object -First 1
    if (-not $asset) {
        $asset = $rel.assets | Where-Object { $_.name -match 'winlibs-x86_64-posix-seh-gcc-.*\.zip$' } | Select-Object -First 1
    }
    if (-not $asset) { throw 'Could not find MinGW-w64 release archive.' }

    $zipPath = Join-Path $ToolsDir 'mingw64.zip'
    Download-MultiThread -Url $asset.browser_download_url -DestPath $zipPath -Threads 8
    Write-Host "[Tools] Extracting MinGW-w64..."
    Expand-Archive -LiteralPath $zipPath -DestinationPath $ToolsDir -Force
    Remove-Item -LiteralPath $zipPath -Force
    if (-not (Test-Path (Join-Path $mingwBin 'gcc.exe'))) {
        throw "MinGW-w64 extraction failed: $mingwBin\gcc.exe not found."
    }
    $env:PATH = "$mingwBin;$env:PATH"
}

$hasMSVC = Find-MSVC
$targetTriple = if ($hasMSVC) { 'x86_64-pc-windows-msvc' } else { 'x86_64-pc-windows-gnu' }

$localCargoBin = Join-Path $ToolsDir 'cargo\bin'
$localRustupHome = Join-Path $ToolsDir 'rustup'
$localCargoHome = Join-Path $ToolsDir 'cargo'

$cargoCmd = $null
if (Test-Path (Join-Path $localCargoBin 'cargo.exe')) {
    $env:RUSTUP_HOME = $localRustupHome
    $env:CARGO_HOME = $localCargoHome
    $env:PATH = "$localCargoBin;$env:PATH"
    $cargoCmd = Join-Path $localCargoBin 'cargo.exe'
} elseif ($c = Get-Command cargo.exe -ErrorAction SilentlyContinue) {
    $cargoCmd = $c.Source
} elseif (Test-Path (Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe')) {
    $userCargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    $env:PATH = "$userCargoBin;$env:PATH"
    $cargoCmd = Join-Path $userCargoBin 'cargo.exe'
}

if (-not $hasMSVC) {
    Ensure-MinGW
}

if (-not $cargoCmd) {
    Write-Host "[Tools] Rust not found. Installing portable Rust into $ToolsDir..."
    $env:RUSTUP_HOME = $localRustupHome
    $env:CARGO_HOME = $localCargoHome
    $rustupInit = Join-Path $ToolsDir 'rustup-init.exe'
    Download-MultiThread -Url 'https://win.rustup.rs/x86_64' -DestPath $rustupInit -Threads 8
    & $rustupInit -y --no-modify-path --default-toolchain "stable-$targetTriple" --profile minimal
    if ($LASTEXITCODE -ne 0) { throw "rustup-init failed with exit code $LASTEXITCODE" }
    $env:PATH = "$localCargoBin;$env:PATH"
    $cargoCmd = Join-Path $localCargoBin 'cargo.exe'
} else {
    Write-Host "[Tools] Using Cargo: $cargoCmd"
    if (-not $hasMSVC) {
        $rustupCmd = Get-Command rustup.exe -ErrorAction SilentlyContinue
        if ($rustupCmd) {
            & $rustupCmd.Source toolchain install "stable-$targetTriple" --profile minimal | Out-Null
            $env:RUSTUP_TOOLCHAIN = "stable-$targetTriple"
        }
    }
}

Write-Host "[Build] Starting multi-threaded build ($Jobs threads)..."
$sw = [System.Diagnostics.Stopwatch]::StartNew()

Push-Location -LiteralPath $Root
try {
    & $cargoCmd build --release --workspace --exclude plugin-launcher -j "$Jobs"
    if ($LASTEXITCODE -ne 0) { throw "Workspace plugins build failed (exit $LASTEXITCODE)" }
} finally {
    Pop-Location
}

Push-Location -LiteralPath (Join-Path $Root 'rust-search')
try {
    & $cargoCmd build --release -j "$Jobs"
    if ($LASTEXITCODE -ne 0) { throw "rust-search build failed (exit $LASTEXITCODE)" }
} finally {
    Pop-Location
}

$payloadDir = Join-Path $Root 'payload'
$testDir = Join-Path $Root 'test-apps'
$testAdvDir = Join-Path $testDir 'system'
New-Item -ItemType Directory -Force -Path $payloadDir, $testDir, $testAdvDir | Out-Null
$systemOnly = @('debloat.exe', 'purge.exe', 'rs-shell.exe', 'reshell.exe')
foreach ($line in Get-Content (Join-Path $Root 'launcher\payload.txt')) {
    if ($line -match '^\s*(#|$)') { continue }
    $name, $src = $line -split '=', 2
    $exeName = $name.Trim()
    $srcPath = Join-Path $Root $src.Trim()
    Copy-Item -LiteralPath $srcPath -Destination (Join-Path $payloadDir $exeName) -Force
    $destTest = if ($systemOnly -contains $exeName) { Join-Path $testAdvDir $exeName } else { Join-Path $testDir $exeName }
    Copy-Item -LiteralPath $srcPath -Destination $destTest -Force
}

$stopBat = @'
@echo off
taskkill /F /IM topbar.exe >nul 2>&1
taskkill /F /IM rs-launcher.exe >nul 2>&1
taskkill /F /IM rust-explorer.exe >nul 2>&1
taskkill /F /IM "Task manager.exe" >nul 2>&1
taskkill /F /IM rust-settings.exe >nul 2>&1
taskkill /F /IM photoview.exe >nul 2>&1
taskkill /F /IM rs.exe >nul 2>&1
if exist "%~dp0topbar.exe" "%~dp0topbar.exe" --restore
echo Da tat het app test va khoi phuc taskbar Windows.
pause
'@
Set-Content -LiteralPath (Join-Path $testDir 'stop-all-test.bat') -Value $stopBat -Encoding ASCII

$previewBat = @'
@echo off
start "" "%~dp0topbar.exe" --preview quick
'@
Set-Content -LiteralPath (Join-Path $testDir 'preview-topbar-quick.bat') -Value $previewBat -Encoding ASCII

Write-Host "[Build] Building plugin-launcher..."
Push-Location -LiteralPath $Root
try {
    & $cargoCmd build --release -p plugin-launcher -j "$Jobs"
    if ($LASTEXITCODE -ne 0) { throw "plugin-launcher build failed (exit $LASTEXITCODE)" }
} finally {
    Pop-Location
}

Copy-Item -LiteralPath (Join-Path $Root 'target\release\plugin-launcher.exe') -Destination (Join-Path $OutDir 'WinRust.exe') -Force
& icacls.exe (Join-Path $OutDir 'WinRust.exe') /setintegritylevel H /Q | Out-Null
Get-ChildItem -LiteralPath $testDir -Filter '*.exe' -Recurse -ErrorAction SilentlyContinue | ForEach-Object {
    & icacls.exe $_.FullName /setintegritylevel M /Q | Out-Null
}

$sw.Stop()
Write-Host ""
Write-Host "[Done] Built in $([math]::Round($sw.Elapsed.TotalSeconds, 1))s -> $OutDir\WinRust.exe" -ForegroundColor Green
Write-Host "[Test] Portable test apps -> $testDir" -ForegroundColor Cyan
} catch {
    Write-Host ""
    Write-Host "[ERROR] $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
