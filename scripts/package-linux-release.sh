#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

version="${1:-$(node -p 'require("./host/package.json").version')}"
public="target/release/public"
server_name="Kestral-${version}-linux-x86_64-server"
server_dir="target/release/${server_name}"

rm -rf "$public" "$server_dir"
mkdir -p "$server_dir/provider-worker/runtime" "$public"
cp target/release/host-server "$server_dir/"
cp host/provider-worker/dist/worker.mjs "$server_dir/provider-worker/"
cp host/provider-worker/runtime/node host/provider-worker/runtime/LICENSE "$server_dir/provider-worker/runtime/"
cp LICENSE THIRD-PARTY-NOTICES.txt "$server_dir/"
cp docs/deployment-modes.md "$server_dir/DEPLOYMENT.md"
tar -C target/release -czf "$public/kestral-${version}-linux-x86_64-server.tar.gz" "$server_name"

appimages=(target/release/bundle/appimage/*"${version}"*.AppImage)
debs=(target/release/bundle/deb/*"${version}"*.deb)
if [[ "${#appimages[@]}" -ne 1 || "${#debs[@]}" -ne 1 ]]; then
  echo "Expected one AppImage and one deb artifact" >&2
  exit 1
fi
cp "${appimages[0]}" "$public/kestral-${version}-linux-x86_64.AppImage"
cp "${debs[0]}" "$public/kestral-${version}-linux-x86_64.deb"
