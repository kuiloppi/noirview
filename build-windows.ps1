param([switch]$IncludeAiRuntime, [switch]$SkipChecks)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
function Run-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" }
}
if (-not $SkipChecks) {
    Run-Cargo fmt --all -- --check
    Run-Cargo clippy --locked --all-targets -- -D warnings
    Run-Cargo test --locked
}
Run-Cargo build --release --locked
$stage = Join-Path $PSScriptRoot 'dist/NoirView-0.1.68-windows-x64'
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target/release/noirview.exe (Join-Path $stage 'NoirView.exe')
Copy-Item target/release/noirview-usn.exe (Join-Path $stage 'noirview-usn.exe')
Copy-Item README.md, LICENSE $stage
[IO.File]::WriteAllText((Join-Path $stage 'portable.marker'), 'portable')

# License texts of the Rust crates built into the program for Windows.
& python tools/third_party_notices.py (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')
if ($LASTEXITCODE -ne 0) { throw 'Cannot write THIRD-PARTY-NOTICES.txt' }

function Add-NativePackage {
    param([string]$Id,[string]$Version,[string[]]$DllEntries)
    $cache = Join-Path $PSScriptRoot 'work/nuget'
    New-Item -ItemType Directory -Force $cache | Out-Null
    $base = "https://api.nuget.org/v3-flatcontainer/$Id/$Version/$Id.$Version.nupkg"
    $package = Join-Path $cache "$Id.$Version.nupkg"
    Invoke-WebRequest $base -OutFile $package
    # NuGet's flat-container API does not serve .nupkg.sha512. The catalog
    # entry publishes the package checksum and its algorithm.
    $registration = Invoke-RestMethod "https://api.nuget.org/v3/registration5-semver1/$Id/$Version.json"
    $catalog = Invoke-RestMethod $registration.catalogEntry
    if ($catalog.packageHashAlgorithm -ne 'SHA512' -or -not $catalog.packageHash) {
        throw "Missing published SHA-512 checksum: $Id"
    }
    $expected = $catalog.packageHash
    $stream = [IO.File]::OpenRead($package)
    try { $sha = [Security.Cryptography.SHA512]::Create(); $actual = [Convert]::ToBase64String($sha.ComputeHash($stream)) }
    finally { $stream.Dispose() }
    if ($actual -ne $expected.Trim()) { throw "Package checksum mismatch: $Id" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($package)
    try {
        foreach ($name in $DllEntries) {
            $entry = $zip.GetEntry($name)
            if ($null -eq $entry) { throw "Missing native library: $name" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $stage ([IO.Path]::GetFileName($name))), $true)
        }
        $licenses = Join-Path $stage "licenses/$Id"
        New-Item -ItemType Directory -Force $licenses | Out-Null
        foreach ($entry in $zip.Entries) {
            if ($entry.Name -match '^(LICENSE|ThirdPartyNotices)' -and $entry.Length -lt 10000000) {
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,(Join-Path $licenses $entry.Name),$true)
            }
        }
    } finally { $zip.Dispose() }
}
if ($IncludeAiRuntime) {
    Add-NativePackage 'microsoft.ml.onnxruntime.directml' '1.22.0' @('runtimes/win-x64/native/onnxruntime.dll','runtimes/win-x64/native/onnxruntime_providers_shared.dll')
    Add-NativePackage 'microsoft.ai.directml' '1.15.4' @('bin/x64-win/DirectML.dll')
}
$archive = "$stage.zip"
Compress-Archive -Path $stage -DestinationPath $archive -Force
Get-FileHash -Algorithm SHA256 $archive
Write-Host "Package: $archive"
