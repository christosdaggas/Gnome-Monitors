#!/usr/bin/env bash
# Builds the monitor-layout RPM locally.
# Requires: rpmdevtools, rust, cargo, gtk4-devel, libadwaita-devel,
#           desktop-file-utils, libappstream-glib
set -euo pipefail

VERSION=1.0.0
NAME=monitor-layout
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOP="${ROOT}/target/rpmbuild"

rm -rf "${TOP}"
mkdir -p "${TOP}"/{SOURCES,SPECS}

# Source tarball (working tree, minus build artifacts). Note: --exclude
# matches the on-disk names, before --transform is applied.
SRCDIR="$(basename "${ROOT}")"
tar -C "${ROOT}/.." \
    --exclude "${SRCDIR}/target" \
    --exclude "${SRCDIR}/dist" \
    --exclude "${SRCDIR}/packaging/vendor" \
    --exclude "${SRCDIR}/.git" \
    --transform "s|^${SRCDIR}|${NAME}-${VERSION}|" \
    -czf "${TOP}/SOURCES/${NAME}-${VERSION}.tar.gz" "${SRCDIR}"

# Vendored dependencies for an offline %build.
( cd "${ROOT}" && cargo vendor --locked packaging/vendor >/dev/null )
tar -C "${ROOT}/packaging" -cJf "${TOP}/SOURCES/${NAME}-${VERSION}-vendor.tar.xz" vendor

cp "${ROOT}/packaging/${NAME}.spec" "${TOP}/SPECS/"

rpmbuild --define "_topdir ${TOP}" -ba "${TOP}/SPECS/${NAME}.spec"

# Collect the results under dist/rpm/.
mkdir -p "${ROOT}/dist/rpm"
find "${TOP}/RPMS" "${TOP}/SRPMS" -name '*.rpm' -exec cp -v {} "${ROOT}/dist/rpm/" \;

( cd "${ROOT}/dist/rpm" && sha256sum *.rpm > SHA256SUMS )

echo
echo "RPMs in ${ROOT}/dist/rpm:"
ls -l "${ROOT}/dist/rpm"
