# Check Windows release files (dsd2pcm.exe, dsd2pcm.dll) for .github/workflows/release.yml:
#
#     .github/scripts/check-windows-signature.ps1 FILE...
#
# Each file must pass `signtool verify /pa /tw` (the default Authenticode policy, chained to a
# trusted root, with a timestamp: /tw turns a missing timestamp into a warning, and anything
# but exit status 0 fails), and PowerShell must report the signature as Valid. signtool comes
# from the newest Windows SDK on the runner. Needs no credentials.
$ErrorActionPreference = "Stop"
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
if (-not $signtool) { throw "signtool.exe not found" }
Write-Output "signtool: $signtool"
foreach ($f in $args) {
    & $signtool verify /pa /tw /v $f
    if ($LASTEXITCODE -ne 0) { throw "signtool verify /pa /tw: $f (exit $LASTEXITCODE)" }
    $sig = Get-AuthenticodeSignature $f
    if ($sig.Status -ne "Valid") { throw "$f : signature $($sig.Status)" }
    if (-not $sig.TimeStamperCertificate) { throw "$f : no timestamp" }
    Write-Output "$f : signed by $($sig.SignerCertificate.Subject), timestamped by $($sig.TimeStamperCertificate.Subject)"
}
