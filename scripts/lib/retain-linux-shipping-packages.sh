#!/usr/bin/env bash
set -euo pipefail

bundle_root="${1:?bundle root is required}"
artifact_root="${2:?artifact root is required}"

set -- "$bundle_root"/deb/*.deb
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "expected exactly one fresh shipping .deb under $bundle_root/deb" >&2
  exit 1
fi
shipping_deb="$1"

set -- "$bundle_root"/rpm/*.rpm
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "expected exactly one fresh shipping .rpm under $bundle_root/rpm" >&2
  exit 1
fi
shipping_rpm="$1"

mkdir -p "$artifact_root"
cp -- "$shipping_deb" "$shipping_rpm" "$artifact_root/"
printf '%s\n' "$artifact_root/$(basename "$shipping_deb")"
