# Lipi (লিপি) - Windows Environment Setup & Verification Script
# Usage: powershell -ExecutionPolicy Bypass -File .\setup-windows.ps1

$ErrorActionPreference = "Continue"

Write-Host "`n=========================================" -ForegroundColor Cyan
Write-Host "    Lipi (লিপি) - Windows Setup Helper  " -ForegroundColor Cyan
Write-Host "=========================================`n" -ForegroundColor Cyan

$allGood = $true

# 1. Check Node.js
Write-Host "[1/5] Checking Node.js..." -NoNewline
if (Get-Command node -ErrorAction SilentlyContinue) {
    $nodeVer = node --version
    Write-Host " OK ($nodeVer)" -ForegroundColor Green
} else {
    Write-Host " MISSING" -ForegroundColor Red
    Write-Host "      Please install Node.js (via nvm, fnm, or https://nodejs.org/)" -ForegroundColor Yellow
    $allGood = $false
}

# 2. Check Python
Write-Host "[2/5] Checking Python 3..." -NoNewline
$pythonExe = $null
foreach ($pyCmd in @("python", "python3", "py")) {
    try {
        $out = & $pyCmd --version 2>&1
        if ($LASTEXITCODE -eq 0 -and $out -match "Python 3") {
            $pythonExe = $pyCmd
            $pyVer = $out
            break
        }
    } catch {}
}

if ($pythonExe) {
    Write-Host " OK ($pyVer via '$pythonExe')" -ForegroundColor Green
} else {
    Write-Host " MISSING / WARNING" -ForegroundColor Yellow
    Write-Host "      Python 3 is recommended for offline 'faster-whisper' engine." -ForegroundColor Yellow
    Write-Host "      Install via: winget install Python.Python.3.11" -ForegroundColor Yellow
}

# 3. Check Rust toolchain
Write-Host "[3/5] Checking Rust (cargo & rustc)..." -NoNewline
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $cargoBin = "$env:USERPROFILE\.cargo\bin"
    if (Test-Path "$cargoBin\cargo.exe") {
        $env:PATH = "$cargoBin;$env:PATH"
    }
}

if (Get-Command cargo -ErrorAction SilentlyContinue) {
    $rustVer = rustc --version
    Write-Host " OK ($rustVer)" -ForegroundColor Green
} else {
    Write-Host " MISSING" -ForegroundColor Red
    Write-Host "      Installing Rust via rustup..." -ForegroundColor Yellow
    $tempRustup = "$env:TEMP\rustup-init.exe"
    curl.exe -sL -o $tempRustup "https://win.rustup.rs/x86_64"
    & $tempRustup -y
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        Write-Host "      Rust successfully installed!" -ForegroundColor Green
    } else {
        $allGood = $false
    }
}

# 4. Check Microsoft Visual C++ Build Tools
Write-Host "[4/5] Checking C++ Build Tools..." -NoNewline
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasMsvc = $false

if (Test-Path $vswhere) {
    $msvcInstall = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($msvcInstall) {
        $hasMsvc = $true
    }
}

if (-not $hasMsvc) {
    $clCmd = Get-Command cl.exe -ErrorAction SilentlyContinue
    if ($clCmd) { $hasMsvc = $true }
}

if ($hasMsvc) {
    Write-Host " OK (MSVC found)" -ForegroundColor Green
} else {
    Write-Host " MISSING / REQUIRED FOR COMPILE" -ForegroundColor Yellow
    Write-Host "      Tauri requires Microsoft C++ Build Tools on Windows." -ForegroundColor Yellow
    Write-Host "      Install via command prompt / PowerShell as Administrator:" -ForegroundColor Yellow
    Write-Host "      winget install Microsoft.VisualStudio.2022.BuildTools --override `"--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`"" -ForegroundColor Cyan
}

# 5. Install Node dependencies
Write-Host "[5/5] Checking Node dependencies..." -NoNewline
if (Test-Path "node_modules") {
    Write-Host " OK (node_modules present)" -ForegroundColor Green
} else {
    Write-Host " Installing..." -ForegroundColor Yellow
    npm install
    if ($LASTEXITCODE -eq 0) {
        Write-Host "      Node dependencies installed successfully." -ForegroundColor Green
    } else {
        Write-Host "      npm install encountered an error." -ForegroundColor Red
        $allGood = $false
    }
}

Write-Host "`n=========================================" -ForegroundColor Cyan
if ($allGood) {
    Write-Host " Setup check finished! Ready to develop." -ForegroundColor Green
    Write-Host " To start Lipi in development mode: npm run tauri dev" -ForegroundColor Cyan
    Write-Host " To build production package:       npm run tauri build" -ForegroundColor Cyan
} else {
    Write-Host " Please resolve any missing items above." -ForegroundColor Yellow
}
Write-Host "=========================================`n" -ForegroundColor Cyan
