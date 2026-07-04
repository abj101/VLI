#!/bin/sh
# Wrap the macOS linker: ad-hoc sign jarvis with stable bundle id + mic entitlements after link.
set -eu

out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ]; then
    out="$arg"
  fi
  prev="$arg"
done

/usr/bin/clang "$@"

if [ -n "$out" ] && [ -f "$out" ]; then
  case "$(basename "$out")" in
    jarvis)
      script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
      entitlements="$script_dir/Entitlements.plist"
      if [ -f "$entitlements" ]; then
        /usr/bin/codesign --force --sign - \
          --identifier com.jarvis.app \
          --entitlements "$entitlements" \
          "$out" >/dev/null 2>&1 || true
      fi
      ;;
  esac
fi
