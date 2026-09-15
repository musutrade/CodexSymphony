#!/bin/bash
# Operator installation of a dedicated outer user-namespace launcher.
set -euo pipefail
if [[ $EUID -ne 0 ]]; then
  echo 'Run with sudo; installing an AppArmor profile requires root.' >&2
  exit 1
fi
expected=0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0
printf '%s  /usr/bin/bwrap\n' "$expected" | sha256sum --check
source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
profile="$source_dir/codexsymphony-bwrap.apparmor"
apparmor_parser -Q -K "$profile"
install -d -o root -g gem -m 0750 /usr/local/libexec/codexsymphony
install -o root -g gem -m 0750 /usr/bin/bwrap /usr/local/libexec/codexsymphony/bwrap
install -o root -g root -m 0644 "$profile" /etc/apparmor.d/codexsymphony-bwrap
apparmor_parser -r /etc/apparmor.d/codexsymphony-bwrap
printf '%s\n' 'Installed dedicated CodexSymphony launcher. Global bwrap and sysctls are unchanged.'
