{
  description = "Editor — Rust-native text editor for Dioxus, inspired by CodeMirror 6.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Reuse the shared Dioxus flake's dev shell — it already bundles
    # the rust toolchain (from rust-toolchain.toml), `dx`, wasm-bindgen,
    # GTK/WebView/etc system deps for desktop, and the wasm32 target.
    # Pointed at the local path that Task uses; switch to
    # `github:FastTrackStudios/Dioxus-Flake` for a clean clone.
    dioxus-flake = {
      url = "path:/home/cody/Development/Dioxus/dioxus-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
    };
  };

  outputs = inputs@{ flake-parts, nixpkgs, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem = { system, ... }:
        let
          pkgs = import nixpkgs { inherit system; };
          dioxusShell = inputs.dioxus-flake.devShells.${system}.default;
        in {
          # Extend the Dioxus shared shell with Node + pnpm +
          # the Playwright-managed Chromium binaries. The
          # `playwright-driver.browsers` derivation ships a
          # Chromium build linked against all the system libs
          # it needs (libnspr4, libnss3, libdrm, libxkbcommon,
          # mesa, etc.) — so we don't have to enumerate them.
          # Setting PLAYWRIGHT_BROWSERS_PATH points
          # @playwright/test at this prebuilt Chromium and
          # PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD prevents npm
          # install-browsers from clobbering it with the
          # generic build that won't link on NixOS.
          devShells.default = pkgs.mkShell {
            inputsFrom = [ dioxusShell ];
            packages = with pkgs; [
              nodejs_22
              pnpm
              playwright-driver.browsers
            ];
            shellHook = ''
              export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
              export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
              # On non-Ubuntu Linuxes Playwright's host check is
              # too strict; this disables it for the nix shell.
              export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
            '';
          };
        };
    };
}
