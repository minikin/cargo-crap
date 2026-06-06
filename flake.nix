{
  description = "cargo-crap — Change Risk Anti-Patterns (CRAP) metric for Rust projects";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
      in
      {
        # `nix build` / `nix run github:minikin/cargo-crap`
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          version = manifest.version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          # The test suite exercises fixtures and the dogfood coverage flow;
          # CI runs it across the OS matrix. Skipping it here keeps `nix build`
          # a pure compile of the tool.
          doCheck = false;

          meta = {
            inherit (manifest) description homepage;
            license = pkgs.lib.licenses.mit;
            mainProgram = "cargo-crap";
          };
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
          name = "cargo-crap";
        };

        # `nix develop` — toolchain plus the helpers the Justfile and CI use.
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            cargo-nextest
            cargo-llvm-cov
            cargo-audit
            cargo-mutants
            just
          ];
        };
      }
    );
}
