{
  description = "sh3rine — stateless S3 static site proxy";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.apple-sdk
            pkgs.libiconv
          ];
        };

        # Build deps-only crate for better layer caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        sh3rine = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });

        # Minimal container image
        dockerImage = pkgs.dockerTools.buildLayeredImage {
          name = "sh3rine";
          tag = "latest";
          contents = [ sh3rine pkgs.cacert ];
          config = {
            Cmd = [ "${sh3rine}/bin/sh3rine" ];
            Env = [
              "RUST_LOG=info"
              "LISTEN=0.0.0.0:8080"
            ];
            ExposedPorts = { "8080/tcp" = {}; };
          };
        };
      in
      {
        packages = {
          default = sh3rine;
          inherit sh3rine dockerImage;
        };

        devShells.default = craneLib.devShell {
          packages = [ pkgs.rust-analyzer ];
        };
      });
}
