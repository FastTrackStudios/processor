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
              # `just` for the convenience recipes in justfile.
              just
              # clang + llvm provide a wasm-capable C cross-
              # compiler so `cc::Build`-based crates (notably
              # arborium-sysroot, which ships the wasm sysroot
              # for tree-sitter grammars) actually emit wasm
              # object files. Without these `cc` silently falls
              # back to host gcc, produces x86-64 ELF objects,
              # the wasm linker ignores them, and the resulting
              # wasm has unresolved `env` imports.
              # Use the *unwrapped* clang/llvm-ar so nix's
              # cc-wrapper doesn't inject host-only hardening
              # flags (e.g. `-fzero-call-used-regs`) that wasm
              # doesn't accept.
              llvmPackages.clang-unwrapped
              llvmPackages.bintools-unwrapped
            ];
            shellHook = ''
              export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
              export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
              export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
              # Pin the `cc` crate's wasm32 C compiler/archiver.
              # arborium-sysroot's build.rs compiles its bundled
              # tree-sitter C runtime through `cc::Build`; without
              # these env vars `cc` falls back to host gcc, the
              # objects are x86-64 ELF, the wasm linker silently
              # drops them, and you get unresolved `env` imports
              # at JS load time.
              export CC_wasm32_unknown_unknown=${pkgs.llvmPackages.clang-unwrapped}/bin/clang
              export AR_wasm32_unknown_unknown=${pkgs.llvmPackages.bintools-unwrapped}/bin/llvm-ar
            '';
          };
        };
    };
}
