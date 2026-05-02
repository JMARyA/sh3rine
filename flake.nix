{
  description = "sh3rine — stateless S3 static site proxy";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      crane,
    }:
    (flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs =
            pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.apple-sdk
              pkgs.libiconv
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              pkgs.openssl
            ];
        };

        # Build deps-only crate for better layer caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        sh3rine = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );

        # Minimal container image
        dockerImage = pkgs.dockerTools.buildLayeredImage {
          name = "sh3rine";
          tag = "latest";
          contents = [
            sh3rine
            pkgs.cacert
          ];
          fakeRootCommands = ''
            mkdir -p etc/ssl
            ln -s ${pkgs.cacert}/etc/ssl/certs etc/ssl/certs
          '';
          config = {
            Cmd = [ "${sh3rine}/bin/sh3rine" ];
            Env = [
              "RUST_LOG=info"
              "LISTEN=0.0.0.0:8080"
              "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              "NIX_SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
            ];
            ExposedPorts = {
              "8080/tcp" = { };
            };
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
      }
    ))
    // {
      moiraPipelines = {
        ci = {
          trigger = {
            on_push.branches = [ "main" ];
            on_pr.target = [ "main" ];
          };
          sandbox_defaults.network = false;
          vars = {
            RUST_BACKTRACE = "1";
            CARGO_TERM_COLOR = "always";
          };
          steps = [
            {
              name = "check";
              env = ./.moira/envs/rust.nix;
              run = "cargo check --workspace";
            }
            {
              name = "test";
              env = ./.moira/envs/rust.nix;
              run = "cargo test --workspace";
              depends_on = [ "check" ];
            }
          ];
        };
        publish = {
          trigger = {
            on_push.branches = [ "main" ];
            manual = true;
          };
          needs_flake = [ "packages.dockerImage" ];
          env = ./.moira/envs/publish.nix;
          steps = [
            {
              name = "push-image";
              run = ''
                skopeo copy \
                  --dest-creds "$REGISTRY_USER:$REGISTRY_PASS" \
                  docker-archive:''${{ flake.packages.dockerImage }} \
                  docker://git.hydrar.de/jmarya/sh3rine:latest
              '';
              sandbox.network = true;
              secrets = [
                "REGISTRY_USER"
                "REGISTRY_PASS"
              ];
            }
          ];
        };
      };
    };
}
