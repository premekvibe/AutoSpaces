# Builds dist\AutoSpaces-Setup-<version>.exe: cargo build --release, then Inno Setup (ISCC).
# Version comes from Cargo.toml. Usage: pwsh tools/build-installer.ps1 [-NoBuild]
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

$m = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^\s*version\s*=\s*"(\d+\.\d+\.\d+)' | Select-Object -First 1
if (-not $m) { throw 'Version not found in Cargo.toml' }
$version = $m.Matches[0].Groups[1].Value

$iscc = @(
    $env:ISCC,
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'ISCC.exe not found (install Inno Setup 6 or set $env:ISCC)' }

Push-Location $root
try {
    if (-not $NoBuild) {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed' }
    }
    & $iscc "/DAppVersion=$version" (Join-Path $root 'installer\autospaces.iss')
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }
    $out = Join-Path $root "dist\AutoSpaces-Setup-$version.exe"
    Write-Host ("{0} ({1:N0} B)" -f $out, (Get-Item $out).Length)
} finally { Pop-Location }
