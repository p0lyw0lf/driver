{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      naersk,
      flake-utils,
      rust-overlay,
    }:
    {
      overlays.default = (final: prev: { inherit (self.packages.${final.system}) driver-bin; });
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rust-toolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        naersk' = pkgs.callPackage naersk {
          cargo = rust-toolchain;
          rustc = rust-toolchain;
        };

        driver-bin = naersk'.buildPackage {
          src = ./.;
          cargoBuildOptions = opts: opts ++ [ "--package driver_bin" ];
        };
      in
      {
        packages = {
          inherit driver-bin;
          default = driver-bin;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [ rust-toolchain ];
          nativeBuildInputs = with pkgs; [
            # For running derivations
            python3
            # For debugging
            vscode-extensions.vadimcn.vscode-lldb.adapter
          ];
        };
      }
    );
}
