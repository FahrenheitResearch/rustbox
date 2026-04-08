$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$upstream = Join-Path $root "upstream"

$repos = @(
    "metrust-py",
    "sharprs",
    "ecape-rs",
    "cfrust",
    "ecrust",
    "rusbie",
    "rustdar",
    "wrf-rust",
    "wrf-rust-plots",
    "geors",
    "geocat-rs",
    "met-cu",
    "open-mrms"
)

New-Item -ItemType Directory -Force $upstream | Out-Null
Push-Location $upstream
try {
    foreach ($repo in $repos) {
        if (Test-Path $repo) {
            Write-Output "skip $repo"
            continue
        }
        git clone "https://github.com/FahrenheitResearch/$repo.git"
    }
} finally {
    Pop-Location
}

