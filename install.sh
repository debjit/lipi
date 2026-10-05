#!/usr/bin/env bash
# Install Lipi from the signed apt archive on Debian and Ubuntu (x86_64).
#   curl -fsSL https://raw.githubusercontent.com/debjit/lipi/main/install.sh | bash
set -euo pipefail

ARCHIVE_URL="https://debjit.github.io/lipi"
KEY_URL="${ARCHIVE_URL}/lipi-archive-keyring.gpg"
RELEASE_URL="${ARCHIVE_URL}/dists/stable/Release"
KEYRING="/etc/apt/keyrings/lipi.gpg"
SOURCE="/etc/apt/sources.list.d/lipi.sources"

run_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo "$@"
  fi
}

if [ "$(uname -m)" != "x86_64" ]; then
  echo "This installer supports Debian and Ubuntu on x86_64 only." >&2
  exit 1
fi

if ! command -v apt-get >/dev/null 2>&1; then
  echo "This installer supports Debian and Ubuntu, which provide apt-get." >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required to download the archive key." >&2
  exit 1
fi

if ! curl -fsSL --output /dev/null "${RELEASE_URL}"; then
  echo "The Lipi apt archive is not published yet. No apt source was added." >&2
  echo "See https://github.com/debjit/lipi/releases" >&2
  exit 1
fi

echo "Adding the Lipi archive key..."
run_root mkdir -p /etc/apt/keyrings
curl -fsSL "${KEY_URL}" | run_root tee "${KEYRING}" >/dev/null
run_root chmod 644 "${KEYRING}"

echo "Adding the Lipi apt source..."
run_root tee "${SOURCE}" >/dev/null <<EOF
Types: deb
URIs: ${ARCHIVE_URL}
Suites: stable
Components: main
Signed-By: ${KEYRING}
Architectures: amd64
EOF

echo "Installing Lipi..."
run_root apt-get update
run_root apt-get install -y lipi

echo "Lipi is installed. Later upgrades are: sudo apt update && sudo apt upgrade"
