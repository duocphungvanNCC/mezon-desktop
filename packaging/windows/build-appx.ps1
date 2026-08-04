param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Exe = "target\release\mezon.exe",
    [string]$OutDir = "target\appx"
)

$ErrorActionPreference = "Stop"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must be MAJOR.MINOR.PATCH, got '$Version'"
}
$packageVersion = "$Version.0"

function Resolve-RepoPath([string]$path) {
    if ([IO.Path]::IsPathRooted($path)) { $path } else { Join-Path (Get-Location) $path }
}

$appxDir = Join-Path $PSScriptRoot "appx"
$exePath = Resolve-RepoPath $Exe
if (-not (Test-Path $exePath)) {
    throw "Executable not found: $exePath (build with 'cargo build --release -p mezon-app' first)"
}

$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction Ignore |
    Sort-Object FullName | Select-Object -Last 1
if (-not $makeappx) {
    throw "makeappx.exe not found; install the Windows 10/11 SDK"
}
$makepri = Join-Path $makeappx.DirectoryName "makepri.exe"
if (-not (Test-Path $makepri)) {
    throw "makepri.exe not found next to $($makeappx.FullName)"
}

$outPath = Resolve-RepoPath $OutDir
$layout = Join-Path $outPath "layout"
Remove-Item -Recurse -Force $layout -ErrorAction Ignore
New-Item -ItemType Directory -Force $layout | Out-Null

Copy-Item $exePath (Join-Path $layout "mezon.exe")
Copy-Item (Join-Path $appxDir "assets") (Join-Path $layout "Assets") -Recurse
$manifest = Get-Content (Join-Path $appxDir "AppxManifest.xml") -Raw
$manifest.Replace("@VERSION@", $packageVersion) |
    Set-Content (Join-Path $layout "AppxManifest.xml") -Encoding utf8

$priconfig = Join-Path $outPath "priconfig.xml"
& $makepri createconfig /cf $priconfig /dq en-US /o
if ($LASTEXITCODE -ne 0) { throw "makepri createconfig failed ($LASTEXITCODE)" }
& $makepri new /pr $layout /cf $priconfig /of (Join-Path $layout "resources.pri") /mn (Join-Path $layout "AppxManifest.xml") /o
if ($LASTEXITCODE -ne 0) { throw "makepri new failed ($LASTEXITCODE)" }

$appx = Join-Path $outPath "Mezon-$Version.appx"
& $makeappx.FullName pack /d $layout /p $appx /o
if ($LASTEXITCODE -ne 0) { throw "makeappx pack failed ($LASTEXITCODE)" }

Write-Host "Store package: $appx (unsigned; the Store signs it on submission)"
