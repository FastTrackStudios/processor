{
  description = "FTS-Audiocore — FastTrackStudio shared audio DSP and plugin core";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    devenv.url = "github:cachix/devenv";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    fts-flake.url = "github:FastTrackStudios/fts-flake";
    fts-flake.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = [
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
      "fasttrackstudio.cachix.org-1:r7v7WXBeSZ7m5meL6w0wttnvsOltRvTpXeVNItcy9f4="
    ];
    extra-substituters = [
      "https://devenv.cachix.org"
      "https://fasttrackstudio.cachix.org"
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      flake-utils,
      rust-overlay,
      fts-flake,
    } @ inputs:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        ftsReaperConfig = "$HOME/.config/FastTrackStudio/Reaper";

        ftsDev = fts-flake.lib.mkFtsPackages {
          inherit pkgs;
          cfg = fts-flake.presets.dev // {
            reaper.configDir = ftsReaperConfig;
          };
        };
      in
      {
        devShells = {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, config, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = with pkgs; [
                    pkg-config
                    openssl
                    libx11 libxi libxext libxrandr libxcursor libxinerama
                    libxcomposite libxdamage libxfixes libxrender libxtst
                    libxcb libxscrnsaver
                    libxkbcommon
                    vulkan-loader vulkan-headers vulkan-tools libGL mesa
                    gtk3 glib gdk-pixbuf pango cairo atk
                    wayland wayland-protocols
                    fontconfig freetype
                    alsa-lib pipewire.jack
                    llvmPackages.libclang
                    dbus zlib stdenv.cc.cc.lib
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
                    LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
                      pkgs.vulkan-loader
                      pkgs.libGL
                      pkgs.wayland
                      pkgs.libxkbcommon
                    ];
                  };

                  scripts = {
                    fts-build.exec = "cargo build --workspace";
                    fts-build.description = "Build FTS-Audiocore workspace";

                    fts-test.exec = "cargo test --workspace";
                    fts-test.description = "Run all unit tests";

                    fts-check.exec = "cargo check --workspace";
                    fts-check.description = "Type-check the workspace";

                    fts-clippy.exec = "cargo clippy --workspace -- -D warnings";
                    fts-clippy.description = "Run clippy lints";
                  };

                  enterShell = ''
                    echo ""
                    echo "  FTS-Audiocore dev shell (devenv + fts-flake)"
                    echo "  ──────────────────────────────────────────"
                    echo "  fts-build     — cargo build --workspace"
                    echo "  fts-test      — cargo test --workspace"
                    echo "  fts-check     — cargo check --workspace"
                    echo "  fts-clippy    — run clippy lints"
                    echo ""
                  '';
                }
              )
            ];
          };
        };
      }
    );
}
