$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$showcaseDir = Join-Path $repoRoot "docs\assets\showcase"
$demoDir = Join-Path $repoRoot "target\demo"
$mesoDir = Join-Path $demoDir "mesoanalysis\png\2024040100_f00"
$radarFixture = Join-Path $repoRoot "tests\fixtures\KATX20240101_000258_partial_V06"

if (Test-Path $showcaseDir) {
    Remove-Item -LiteralPath $showcaseDir -Recurse -Force
}
New-Item -ItemType Directory -Path $showcaseDir | Out-Null

Push-Location $repoRoot
try {
    cargo run -p wx-cli -- demo | Out-Host
    cargo run -p mesoanalysis-app -- demo | Out-Host
    cargo run -p radar-viewer-app -- render $radarFixture REF (Join-Path $demoDir "radar_reflectivity.png") 0 512 classic default | Out-Host
}
finally {
    Pop-Location
}

Copy-Item -LiteralPath (Join-Path $demoDir "hrrr_gust_surface_basemap.png") -Destination (Join-Path $showcaseDir "hrrr_gust_surface_basemap.png")
Copy-Item -LiteralPath (Join-Path $demoDir "hrrr_model_sounding.png") -Destination (Join-Path $showcaseDir "hrrr_model_sounding.png")
Copy-Item -LiteralPath (Join-Path $demoDir "hrrr_gust_surface_overlay.png") -Destination (Join-Path $showcaseDir "hrrr_gust_surface_overlay.png")
Copy-Item -LiteralPath (Join-Path $mesoDir "smoothed_vorticity_850mb.png") -Destination (Join-Path $showcaseDir "smoothed_vorticity_850mb.png")
Copy-Item -LiteralPath (Join-Path $mesoDir "divergence_850mb.png") -Destination (Join-Path $showcaseDir "divergence_850mb.png")
Copy-Item -LiteralPath (Join-Path $mesoDir "temperature_advection_850mb.png") -Destination (Join-Path $showcaseDir "temperature_advection_850mb.png")
Copy-Item -LiteralPath (Join-Path $mesoDir "frontogenesis_850mb.png") -Destination (Join-Path $showcaseDir "frontogenesis_850mb.png")
Copy-Item -LiteralPath (Join-Path $demoDir "radar_reflectivity.png") -Destination (Join-Path $showcaseDir "radar_reflectivity.png")

Get-ChildItem $showcaseDir | Select-Object Name, CreationTime, LastWriteTime, Length
