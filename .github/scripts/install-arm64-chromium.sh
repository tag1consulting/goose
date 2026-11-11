#!/usr/bin/env bash
# shellcheck disable=SC2329 # the try_ functions are called by name, at the end

# Installs Chromium on an arm64 Ubuntu 24.04 runner so that `chromium` is on
# PATH, for the PDF report tests that prove Goose finds an installed browser.
#
# Tries each source in turn and stops at the first that gives a working
# `chromium --version`:
#   1. playwright: the Chrome for Testing arm64 build from Playwright's CDN,
#      unpacked into CHROMIUM_DIR (cached by actions/cache) and linked as
#      /usr/local/bin/chromium
#   2. debian:     Debian trixie's chromium .deb from deb.debian.org
#   3. ppa:        ppa:xtradeb/apps
#   4. snap:       the Snap Store
# Every network step has a short timeout, so a dead host fails fast. Fails only
# if all four fail.
#
# CHROMIUM_VERSION  the Chrome for Testing version to download (required)
# CHROMIUM_DIR      where source 1 unpacks it (default ~/.cache/ci-chromium)
set -uo pipefail

: "${CHROMIUM_VERSION:?set CHROMIUM_VERSION, such as 158.0.8127.18}"
CHROMIUM_DIR=${CHROMIUM_DIR:-$HOME/.cache/ci-chromium}
PLAYWRIGHT_URL="https://cdn.playwright.dev/builds/cft/$CHROMIUM_VERSION/linux-arm64/chrome-linux-arm64.zip"

as_root() {
  if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi
}

APT=(-y -q -o Acquire::Retries=1 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)

# Libraries the Chrome for Testing build needs, Playwright's list for Ubuntu 24.04.
CHROME_LIBS="libasound2t64 libatk-bridge2.0-0t64 libatk1.0-0t64 libatspi2.0-0t64
  libcairo2 libcups2t64 libdbus-1-3 libdrm2 libgbm1 libglib2.0-0t64 libnspr4
  libnss3 libpango-1.0-0 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6
  libxfixes3 libxkbcommon0 libxrandr2"

apt_update() {
  as_root timeout 180 apt-get "${APT[@]}" update
}

# Passes if a chromium on PATH runs.
works() {
  command -v chromium >/dev/null && timeout 60 chromium --version
}

try_playwright() {
  local bin=$CHROMIUM_DIR/chrome-linux-arm64/chrome
  if [ -x "$bin" ]; then
    echo "Using the cached Chrome for Testing $CHROMIUM_VERSION in $CHROMIUM_DIR"
  else
    local zip
    zip=$(mktemp --suffix=.zip)
    rm -rf "$CHROMIUM_DIR"
    mkdir -p "$CHROMIUM_DIR"
    curl -fsSL --connect-timeout 15 --max-time 300 -o "$zip" "$PLAYWRIGHT_URL" &&
      unzip -q "$zip" -d "$CHROMIUM_DIR"
    local status=$?
    rm -f "$zip"
    [ "$status" = 0 ] || return 1
  fi
  if ldd "$bin" | grep 'not found'; then
    # shellcheck disable=SC2086 # CHROME_LIBS is a list of package names
    apt_update && as_root timeout 300 apt-get "${APT[@]}" install --no-install-recommends $CHROME_LIBS || return 1
  fi
  as_root ln -sf "$bin" /usr/local/bin/chromium
}

try_debian() {
  local list=/etc/apt/sources.list.d/ci-debian-trixie.sources
  local pin=/etc/apt/preferences.d/ci-debian-trixie
  apt_update || return 1
  as_root timeout 300 apt-get "${APT[@]}" install --no-install-recommends debian-archive-keyring || return 1
  as_root tee "$list" >/dev/null <<'EOF'
Types: deb
URIs: http://deb.debian.org/debian
Suites: trixie
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg

Types: deb
URIs: http://deb.debian.org/debian-security
Suites: trixie-security
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
EOF
  # Debian's chromium over Ubuntu's snap wrapper. Nothing else comes from
  # Debian unless chromium needs it, and then from trixie (-t trixie): it
  # needs a newer libharfbuzz0b and libxcomposite1 than noble has.
  as_root tee "$pin" >/dev/null <<'EOF'
Package: chromium chromium-common chromium-sandbox
Pin: origin deb.debian.org
Pin-Priority: 1001

Package: *
Pin: origin deb.debian.org
Pin-Priority: 100
EOF
  if apt_update && as_root timeout 300 apt-get "${APT[@]}" install --no-install-recommends -t trixie chromium; then
    apt-cache policy chromium
    return 0
  fi
  as_root rm -f "$list" "$pin"
  return 1
}

try_ppa() {
  local pin=/etc/apt/preferences.d/ci-xtradeb
  # add-apt-repository asks Launchpad for the PPA's key, so it fails when
  # Launchpad is down.
  as_root timeout 60 add-apt-repository -y -n ppa:xtradeb/apps || { remove_ppa; return 1; }
  # The PPA's chromium over Ubuntu's snap wrapper.
  as_root tee "$pin" >/dev/null <<'EOF'
Package: chromium*
Pin: release o=LP-PPA-xtradeb-apps
Pin-Priority: 1001
EOF
  if apt_update && as_root timeout 300 apt-get "${APT[@]}" install --no-install-recommends chromium; then
    apt-cache policy chromium
    return 0
  fi
  remove_ppa
  return 1
}

remove_ppa() {
  as_root rm -f /etc/apt/sources.list.d/xtradeb-ubuntu-apps-* /etc/apt/trusted.gpg.d/xtradeb-ubuntu-apps.gpg \
    /etc/apt/preferences.d/ci-xtradeb
}

try_snap() {
  as_root timeout 180 snap install chromium
}

for source in playwright debian ppa snap; do
  echo "::group::Chromium from $source"
  if "try_$source" && works; then
    echo "::endgroup::"
    echo "::notice title=Chromium source::installed from $source ($(chromium --version))"
    exit 0
  fi
  # Leave no broken link to the playwright build in front of a later source.
  [ "$source" = playwright ] && as_root rm -f /usr/local/bin/chromium
  echo "::endgroup::"
  echo "::warning title=Chromium source::$source failed, trying the next source"
done

echo "::error title=Chromium source::every source failed: playwright, debian, ppa, snap"
exit 1
