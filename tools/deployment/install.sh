#!/bin/sh
# Administrator installation only. Never starts services or changes public ingress.
set -eu
test "$(id -u)" = 0 || { echo 'Run on the target host as administrator' >&2; exit 1; }
test "$#" = 1 || { echo 'Usage: install.sh RELEASE_ID' >&2; exit 1; }
case "$1" in ''|*[!a-zA-Z0-9_-]*) echo 'Invalid release ID' >&2; exit 1;; esac
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
release=/opt/codexsymphony-m2/releases/$1
test ! -e "$release" || { echo 'Release already exists; refusing overwrite' >&2; exit 1; }
install -d -m 0755 "$release"
install -m 0755 "$source_dir/apps/server/deployment/executor.py" "$release/executor.py"
install -m 0755 "$source_dir/tools/deployment/check_ingress.py" "$release/check_ingress.py"
install -d -m 0700 "$release/examples"
install -m 0600 "$source_dir"/deploy/m2/* "$release/examples/"
for role in coding validation; do
    cat > "$release/$role" <<EOF
#!/bin/sh
exec /usr/bin/python3 -I '$release/executor.py' '/etc/codexsymphony-control/$role-boundary.json' "\$@"
EOF
    chmod 0755 "$release/$role"
done
sha256sum "$release/executor.py" "$release/coding" "$release/validation"
echo "Staged $release. Configure and check before switching; no service was changed."
