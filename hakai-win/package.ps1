# Builds hakai-win and assembles the portable release zip.
#
#   powershell -File hakai-win/package.ps1      (Windows PowerShell 5.1 is fine)
#
# Output: hakai-win/target/hakai-win-<version>-x86_64-windows.zip — the stripped release
# binary plus LICENSE and CREDITS.md. hakai-win.exe is fully self-contained (fonts and
# all 35 sounds are compiled in), so there is nothing else to bundle.

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    cargo build --release --manifest-path hakai-win/Cargo.toml --locked

    $ver = [regex]::Match((Get-Content hakai-win/Cargo.toml -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $ver) { throw "couldn't read version from hakai-win/Cargo.toml" }

    $name  = "hakai-win-$ver-x86_64-windows"
    $stage = Join-Path $repo "hakai-win/target/$name"
    $zip   = Join-Path $repo "hakai-win/target/$name.zip"

    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
    Remove-Item -Force $zip -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $stage | Out-Null

    Copy-Item hakai-win/target/release/hakai-win.exe $stage/
    Copy-Item hakai-win/README.txt                   (Join-Path $stage 'README.txt')
    Copy-Item LICENSE                                (Join-Path $stage 'LICENSE.txt')
    Copy-Item CREDITS.md                             $stage/

    Compress-Archive -Path "$stage/*" -DestinationPath $zip -CompressionLevel Optimal

    $mb = "{0:N1}" -f ((Get-Item $zip).Length / 1MB)
    Write-Host "-> $zip  ($mb MB)"
}
finally {
    Pop-Location
}
