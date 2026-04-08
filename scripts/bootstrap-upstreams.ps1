$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$upstream = Join-Path $root "upstream"
$lockPath = Join-Path $root "upstream-lock.json"
$lock = Get-Content $lockPath -Raw | ConvertFrom-Json

New-Item -ItemType Directory -Force $upstream | Out-Null
foreach ($repo in $lock.repos) {
    $repoPath = Join-Path $upstream $repo.name
    if (-not (Test-Path $repoPath)) {
        git clone $repo.url $repoPath
    }
    git -C $repoPath fetch --all --tags --prune
    git -C $repoPath checkout $repo.commit
    Write-Output ("checked out {0} @ {1}" -f $repo.name, $repo.commit)
}

