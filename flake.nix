{
  description = "tuitor: a terminal debugger for every language, built on the Debug Adapter Protocol";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
    in
    {
      packages = forAll (pkgs: rec {
        tuitor = pkgs.rustPlatform.buildRustPackage {
          pname = "tuitor";
          inherit (cargoToml.workspace.package) version;
          # Only what builds the binary, so that a change to the docs or examples rebuilds nothing.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "tuitor" ];
          # The tests start gdb, git and language servers, which the sandbox does not have;
          # CI runs them (.github/workflows/ci.yml).
          doCheck = false;
          meta = {
            description = "Terminal debugger for every language, built on the Debug Adapter Protocol";
            homepage = "https://doriandevp.github.io/tuitor/";
            license = pkgs.lib.licenses.mit;
            mainProgram = "tuitor";
            platforms = pkgs.lib.platforms.unix;
          };
        };
        default = tuitor;
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/tuitor";
          meta.description = "tuitor, the terminal debugger";
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages =
            with pkgs;
            [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              mdbook
              vhs
            ]
            # gdb's DAP server, for the tests; nixpkgs builds gdb for Linux only.
            ++ lib.optionals stdenv.hostPlatform.isLinux [ gdb ];
        };
      });
    };
}
