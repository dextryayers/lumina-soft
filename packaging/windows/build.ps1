[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Arch = "x86_64",
    [string]$MsiVersion = "",
    [switch]$SkipBuild,
    [switch]$SkipMsi,
    [switch]$SkipNsis,
    [switch]$SkipPortable
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Here = $PSScriptRoot
$Root = Split-Path -Parent (Split-Path -Parent $Here)
$Dist = Join-Path $Root "dist"
$Stage = Join-Path $Dist "windows-stage"
$TargetRelease = Join-Path $Root "target\release"

function Get-WorkspaceVersion {
    $manifest = Get-Content -LiteralPath (Join-Path $Root "Cargo.toml") -Raw
    $match = [regex]::Match($manifest, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) {
        throw "Cannot read workspace.package version from Cargo.toml"
    }
    return $match.Groups[1].Value
}

function ConvertTo-MsiVersion([string]$Value) {
    $core = $Value -replace '[-+].*$', ''
    $parts = $core.Split('.')
    if ($parts.Count -lt 3) {
        throw "Version '$Value' is not a three-part version"
    }
    $revision = 0
    if ($Value -match 'alpha\.(\d+)') {
        $revision = [int]$Matches[1]
    } elseif ($Value -match 'beta\.(\d+)') {
        $revision = 100 + [int]$Matches[1]
    } elseif ($Value -match 'rc\.(\d+)') {
        $revision = 200 + [int]$Matches[1]
    }
    return "$($parts[0]).$($parts[1]).$($parts[2]).$revision"
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = Get-WorkspaceVersion
}
if ([string]::IsNullOrWhiteSpace($MsiVersion)) {
    $MsiVersion = ConvertTo-MsiVersion $Version
}

Write-Host "Lumina $Version ($Arch)"
Write-Host "MSI version mapping: $MsiVersion"

if (-not $SkipBuild) {
    Push-Location $Root
    try {
        cargo build --release --locked -p lumina-app -p lumina-ipc
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

$Lumina = Join-Path $TargetRelease "lumina.exe"
$Send = Join-Path $TargetRelease "lumina-send.exe"
foreach ($binary in @($Lumina, $Send)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "Missing release binary: $binary"
    }
}

if (Test-Path -LiteralPath $Stage) {
    Remove-Item -LiteralPath $Stage -Recurse -Force
}
New-Item -ItemType Directory -Path $Stage -Force | Out-Null
New-Item -ItemType Directory -Path $Dist -Force | Out-Null

Copy-Item -LiteralPath $Lumina -Destination $Stage
Copy-Item -LiteralPath $Send -Destination $Stage
foreach ($file in @("LICENSE", "README.md", "CHANGELOG.md")) {
    $path = Join-Path $Root $file
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        Copy-Item -LiteralPath $path -Destination $Stage
    }
}

$Artifacts = New-Object System.Collections.Generic.List[string]

if (-not $SkipPortable) {
    $zip = Join-Path $Dist "Lumina-$Version-$Arch-portable.zip"
    if (Test-Path -LiteralPath $zip) {
        Remove-Item -LiteralPath $zip -Force
    }
    Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $zip -CompressionLevel Optimal
    $Artifacts.Add($zip)
    Write-Host "Built $zip"
}

if (-not $SkipMsi) {
    $wix = Get-Command wix -ErrorAction SilentlyContinue
    if ($null -eq $wix) {
        throw "WiX v4 'wix' was not found on PATH. Install WiX v4 or pass -SkipMsi."
    }
    $msi = Join-Path $Dist "Lumina-$Version-$Arch.msi"
    if (Test-Path -LiteralPath $msi) {
        Remove-Item -LiteralPath $msi -Force
    }
    & $wix.Source build "-dVersion=$MsiVersion" "-dStageDir=$Stage" -o $msi (Join-Path $Here "lumina.wxs")
    if ($LASTEXITCODE -ne 0) {
        throw "WiX failed with exit code $LASTEXITCODE"
    }
    $Artifacts.Add($msi)
    Write-Host "Built $msi"
}

if (-not $SkipNsis) {
    $makensis = Get-Command makensis -ErrorAction SilentlyContinue
    if ($null -eq $makensis) {
        throw "NSIS 'makensis' was not found on PATH. Install NSIS or pass -SkipNsis."
    }
    $setup = Join-Path $Dist "Lumina-$Version-$Arch-setup.exe"
    if (Test-Path -LiteralPath $setup) {
        Remove-Item -LiteralPath $setup -Force
    }
    & $makensis.Source "/DVERSION=$Version" "/DNUMERIC_VERSION=$MsiVersion" "/DSTAGE_DIR=$Stage" "/DOUT_FILE=$setup" (Join-Path $Here "installer.nsi")
    if ($LASTEXITCODE -ne 0) {
        throw "makensis failed with exit code $LASTEXITCODE"
    }
    $Artifacts.Add($setup)
    Write-Host "Built $setup"
}

$checksums = Join-Path $Dist "SHA256SUMS-windows.txt"
$lines = foreach ($artifact in $Artifacts) {
    $hash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $(Split-Path -Leaf $artifact)"
}
$lines | Set-Content -LiteralPath $checksums -Encoding ASCII
Write-Host "Wrote $checksums"
