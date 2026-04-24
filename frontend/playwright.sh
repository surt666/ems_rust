#!/usr/bin/env bash

# Manually build library paths from each package
LIBS=(
  "$(nix-build --no-out-link '<nixpkgs>' -A glib)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A nss)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A nspr)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A dbus)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A atk)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A at-spi2-atk)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A cups)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A expat)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A libxcb)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A libxkbcommon)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A xorg.libX11)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A xorg.libXcomposite)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A xorg.libXdamage)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A xorg.libXext)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A xorg.libXfixes)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A xorg.libXrandr)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A mesa)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A mesa.drivers)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A cairo)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A pango)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A systemd)/lib"
  "$(nix-build --no-out-link '<nixpkgs>' -A alsa-lib)/lib"
)

# Join paths with colons
export LD_LIBRARY_PATH=$(IFS=:; echo "${LIBS[*]}")${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true

npx playwright "$@"
# #!/usr/bin/env bash

# # Build a symlink tree with all required libraries
# PLAYWRIGHT_LIBS=$(nix-build --no-out-link '<nixpkgs>' --expr '
#   with import <nixpkgs> {};
#   symlinkJoin {
#     name = "playwright-libs";
#     paths = [
#       glib nss nspr dbus atk at-spi2-atk cups expat
#       libxcb libxkbcommon xorg.libX11 xorg.libXcomposite
#       xorg.libXdamage xorg.libXext xorg.libXfixes xorg.libXrandr
#       mesa cairo pango systemd alsa-lib
#     ];
#   }
# ')

# export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}${PLAYWRIGHT_LIBS}/lib"
# export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true

# npx playwright "$@"
