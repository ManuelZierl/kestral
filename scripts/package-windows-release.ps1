param(
    [string]$Version = (node -p "require('./host/package.json').version")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$public = "target/release/public"
$portable = "target/release/Kestral-$Version-windows-x86_64"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $public, $portable
New-Item -ItemType Directory -Force -Path "$portable/provider-worker/runtime", $public | Out-Null

Copy-Item "target/release/kestral.exe", "target/release/host-server.exe", "LICENSE", "THIRD-PARTY-NOTICES.txt" -Destination $portable
Copy-Item "docs/getting-started.md" -Destination "$portable/GETTING-STARTED.md"
Copy-Item "host/provider-worker/dist/worker.mjs" -Destination "$portable/provider-worker"
Copy-Item "host/provider-worker/runtime/node.exe", "host/provider-worker/runtime/LICENSE" -Destination "$portable/provider-worker/runtime"
Compress-Archive -Path "$portable/*" -DestinationPath "$public/kestral-$Version-windows-x86_64-portable.zip"

$nsis = @(Get-ChildItem "target/release/bundle/nsis/*_${Version}_*-setup.exe")
if ($nsis.Count -ne 1) {
    throw "Expected one NSIS artifact"
}
Copy-Item -LiteralPath $nsis[0].FullName -Destination "$public/kestral-$Version-windows-x86_64-nsis.exe"
