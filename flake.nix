{
  description = "FastTrackStudio Site — Dioxus documentation and chart browser";

  inputs = {
    dioxus-flake.url = "git+file:///home/cody/Development/Dioxus/dioxus-flake";
    nixpkgs.follows = "dioxus-flake/nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = [
      "fasttrackstudio.cachix.org-1:r7v7WXBeSZ7m5meL6w0wttnvsOltRvTpXeVNItcy9f4="
    ];
    extra-substituters = [
      "https://fasttrackstudio.cachix.org"
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      dioxus-flake,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSystem =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f {
            inherit system;
            pkgs = nixpkgs.legacyPackages.${system};
          }
        );
    in
    {
      packages = forEachSystem (
        { system, pkgs }:
        let
          appName = "fasttrackstudio";
          deploy = dioxus-flake.lib.${system};
          bundleEnv = builtins.getEnv "DX_BUNDLE_DIR";
          bundlePath = if bundleEnv != "" then /. + bundleEnv else ./bundle;
          image = deploy.mkDioxusWebImage {
            inherit appName bundlePath;
            # In-cluster registry on the starcommand k3s host (unauthenticated,
            # plain HTTP, LAN-only) — same registry/pattern Task's images use.
            imageName = "registry.starcommand.live:30050/fasttrackstudio-site";
            # This is a plain client-side SPA (no `fullstack` feature, no
            # server binary in the `dx bundle` output) — unlike
            # mkDioxusWebImage's default assumption of a fullstack app with
            # an `/app/server` binary. Serve the static bundle directly with
            # miniserve instead, with --spa so client-side routes (the
            # Dioxus Router's history-API paths) fall back to index.html
            # instead of 404ing.
            entrypoint = [
              "${pkgs.miniserve}/bin/miniserve"
              "/app/public"
              "--index"
              "index.html"
              "--spa"
              "-p"
              "8080"
            ];
          };
        in
        {
          inherit image;
          default = image;
        }
      );

      apps = forEachSystem (
        { system, pkgs }:
        dioxus-flake.lib.${system}.mkDioxusWebDeployApps {
          appName = "fasttrackstudio";
          dxPackageName = "fasttrackstudio-web";
          imagePackage = "image";
          registry = "registry.fly.io";
        }
      );

      devShells = forEachSystem (
        { system, pkgs }:
        {
          default = pkgs.mkShell {
            inputsFrom = [ dioxus-flake.devShells.${system}.default ];

            shellHook = dioxus-flake.lib.${system}.mkDioxusWebBundleShellHook {
              appName = "fasttrackstudio";
              dxPackageName = "fasttrackstudio-web";
            };
          };
        }
      );
    };
}
