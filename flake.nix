{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      pkgs = import nixpkgs {
        system = "x86_64-linux"; # Replace with your target system if necessary
      };
      oxBuild = pkgs.rustPlatform.buildRustPackage rec {
        pname = "ox";
        version = "v0.1.1-alpha";

        src = pkgs.fetchFromGitHub {
          owner = "pagedMov";
          repo = "ox";
          rev = version;
          hash = "sha256-5XwZmsJF/imB8ZSBM9LCrQRRrG5sbjKl6N7MVYIUIck=";
        };

        doCheck = false; # TODO: Find a way to make tests work

        cargoHash = "sha256-6i+0jQcZdmtoUODDD361NnSsYyxQss24ZkV0JUVAikY=";

        nativeBuildInputs = [
          pkgs.openssl
          pkgs.openssl.dev
          pkgs.pkg-config
        ];

        PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
        passthru = {
          shellPath = "/bin/ox";
        };
      };
    in
    {
      packages.${pkgs.system}.default = oxBuild;

      devShells.default = pkgs.mkShell {
        nativeBuildInputs = [
          pkgs.rust-bin.stable.latest.default
          pkgs.gcc
          pkgs.clang
          pkgs.pkg-config
          pkgs.libgit2
          pkgs.libssh2
          pkgs.libssh2.dev
          pkgs.openssl
          pkgs.openssl.dev
          pkgs.llvm
          pkgs.libclang
          pkgs.pam
        ];

        shellHook = ''
          exec ox
        '';
      };
    };
}
