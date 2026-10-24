#!/bin/sh
# Stand-in archiver to go with cc.sh: creates every archive it is given, empty.
for arg in "$@"; do
  case "$arg" in
    -out:*|/OUT:*) : > "${arg#*:}" ;;
    *.a|*.lib) : > "$arg" ;;
  esac
done
exit 0
