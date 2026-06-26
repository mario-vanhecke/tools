# Install the rag toolkit (rag + md) from the latest GitHub release.
#
# Usage:
#   irm https://github.com/mario-vanhecke/tools/raw/main/install.ps1 | iex
#
# Environment overrides:
#   $env:RAG_VERSION    pin a specific version (default: latest)
#   $env:RAG_PREFIX     install dir (default: %LOCALAPPDATA%\rag\bin)
#   $env:RAG_TOOLS      comma-separated list (default: "rag,md")

#Requires -Version 5

$ErrorActionPreference = 'Stop'

$repo    = 'mario-vanhecke/tools'
$version = if ($env:RAG_VERSION) { $env:RAG_VERSION } else { 'latest' }
$tools   = if ($env:RAG_TOOLS)   { $env:RAG_TOOLS -split ',' | ForEach-Object { $_.Trim() } } else { @('rag','md','crawl','distill','recall') }

function Write-Ok    ($msg) { Write-Host "ok    $msg"   -ForegroundColor Green }
function Write-Note  ($msg) { Write-Host "note  $msg"   -ForegroundColor Yellow }
function Write-Err   ($msg) { Write-Host "error $msg"   -ForegroundColor Red; exit 1 }
function Write-Bold  ($msg) { Write-Host $msg -ForegroundColor White }

# ---------- detect arch ----------
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' { Write-Err "Windows on ARM64 is not yet packaged; build from source via 'cargo install --git https://github.com/$repo'" }
    default { Write-Err "unsupported architecture: $arch" }
}

Write-Bold "Installing rag toolkit ($($tools -join ', ')) for Windows ($arch)"

# ---------- install prefix ----------
$prefix = if ($env:RAG_PREFIX) { $env:RAG_PREFIX } else { Join-Path $env:LOCALAPPDATA 'rag\bin' }
New-Item -ItemType Directory -Force -Path $prefix | Out-Null
Write-Ok "install prefix: $prefix"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("rag-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    foreach ($tool in $tools) {
        if ($version -eq 'latest') {
            $assetUrl = "https://github.com/$repo/releases/latest/download/$tool-$target.zip"
        } else {
            $assetUrl = "https://github.com/$repo/releases/download/$version/$tool-$target.zip"
        }

        $zip = Join-Path $tmp "$tool.zip"
        $extractTo = Join-Path $tmp "$tool-extract"
        New-Item -ItemType Directory -Force -Path $extractTo | Out-Null

        Write-Ok "downloading $tool from $assetUrl"
        try {
            Invoke-WebRequest -Uri $assetUrl -OutFile $zip -UseBasicParsing
        } catch {
            Write-Err "download failed. check that a release exists at $assetUrl"
        }

        Expand-Archive -LiteralPath $zip -DestinationPath $extractTo -Force

        $binSrc = $null
        foreach ($candidate in @((Join-Path $extractTo "$tool.exe"), (Join-Path $extractTo "$tool-$target\$tool.exe"))) {
            if (Test-Path $candidate) { $binSrc = $candidate; break }
        }
        if (-not $binSrc) { Write-Err "binary '$tool.exe' not found inside the zip" }

        Move-Item -Force -Path $binSrc -Destination (Join-Path $prefix "$tool.exe")
        Write-Ok "installed: $prefix\$tool.exe"
    }
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# ---------- PATH hint ----------
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$onPath   = ($userPath -split ';') -contains $prefix
if (-not $onPath) {
    Write-Note "$prefix is not on your PATH. Adding it for the current user."
    $newPath = if ($userPath) { "$userPath;$prefix" } else { $prefix }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Note "Open a new terminal for the change to take effect."
}

if (-not (Get-Command pandoc -ErrorAction SilentlyContinue)) {
    Write-Note "pandoc is not installed. DOCX/EPUB support requires it."
    Write-Host  "        winget install pandoc" -ForegroundColor DarkGray
}
if (-not (Get-Command pdftotext -ErrorAction SilentlyContinue)) {
    Write-Note "poppler is not installed. PDF extraction uses a pure-Rust fallback without it."
    Write-Host  "        choco install poppler" -ForegroundColor DarkGray
}

Write-Host ''
Write-Bold 'Next:'
if ($tools -contains 'rag') {
    Write-Host '  rag --version'
    Write-Host '  rag init . && rag add <path> && rag index && rag search "<query>"'
}
if ($tools -contains 'md') {
    Write-Host '  md --version'
    Write-Host '  md init . && md add <path> && md convert'
}
if ($tools -contains 'crawl') {
    Write-Host '  crawl --version'
    Write-Host '  crawl init . ; crawl source add docs local ./docs ; crawl run ; crawl ls'
}
if ($tools -contains 'distill') {
    Write-Host '  distill --version'
    Write-Host '  distill init ; distill build ; distill search "<query>"'
}
if ($tools -contains 'recall') {
    Write-Host '  recall --version'
    Write-Host '  recall serve knowledge.kb --stdio   # add it as an MCP server in your harness'
}
