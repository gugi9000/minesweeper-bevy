[CmdletBinding()]
param(
    [string]$ServerRoot = (Join-Path $PSScriptRoot '..\..\wasd5'),
    [switch]$DebugBuild
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$server = (Resolve-Path $ServerRoot).Path
if (-not (Test-Path (Join-Path $server 'static') -PathType Container)) {
    throw "ServerRoot must contain the wasd5 static directory."
}

Push-Location $repository
try {
    if (-not (Get-Command wasm-bindgen -ErrorAction SilentlyContinue)) {
        throw 'wasm-bindgen is missing. Install the wasm-bindgen-cli version matching Cargo.lock (see README).'
    }
    $metadata = cargo metadata --format-version 1 --locked --filter-platform wasm32-unknown-unknown
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
    $packages = ($metadata | ConvertFrom-Json).packages
    $version = ($packages | Where-Object name -eq 'wasm-bindgen').version
    $installed = (& wasm-bindgen --version) -replace '^wasm-bindgen\s+', ''
    if ($installed.Trim() -ne $version) {
        throw "Install matching tools: cargo install wasm-bindgen-cli --version $version --locked"
    }

    $profile = if ($DebugBuild) { 'debug' } else { 'release' }
    $buildArgs = @('build', '--locked', '--target', 'wasm32-unknown-unknown')
    if (-not $DebugBuild) { $buildArgs += '--release' }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'WASM build failed.' }

    $staging = Join-Path $repository 'target\makibishi-web'
    New-Item -ItemType Directory -Force $staging | Out-Null
    & wasm-bindgen (Join-Path $repository "target\wasm32-unknown-unknown\$profile\minesweeper-bevy.wasm") `
        --target web --out-dir $staging --out-name makibishi --no-typescript
    if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed.' }
    Copy-Item (Join-Path $repository 'gfx') $staging -Recurse -Force
    Copy-Item (Join-Path $repository 'sfx') $staging -Recurse -Force

    $destination = Join-Path $server 'static\makibishi'
    New-Item -ItemType Directory -Force $destination | Out-Null
    Copy-Item (Join-Path $staging 'makibishi.js') $destination -Force
    Copy-Item (Join-Path $staging 'makibishi_bg.wasm') $destination -Force
    Copy-Item (Join-Path $staging 'gfx') $destination -Recurse -Force
    Copy-Item (Join-Path $staging 'sfx') $destination -Recurse -Force
    Write-Host "Makibishi deployed to $destination. Open /games/makibishi on wasd5."
}
finally {
    Pop-Location
}
