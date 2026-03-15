{
  description = "pipemeeter Rust app";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    crane,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
      };
      llvmPkgs = pkgs.llvmPackages;
      runtimeLibs = [
        pkgs.wayland
        pkgs.libxkbcommon
        pkgs.libx11
        pkgs.libxcursor
        pkgs.libxi
        pkgs.libxrandr
        pkgs.libxinerama
        pkgs.libxcb
        pkgs.libGL
        pkgs.vulkan-loader
        pkgs.pipewire
      ];

      craneLib = crane.mkLib pkgs;
      src = craneLib.cleanCargoSource ./.;

      commonArgs = {
        inherit src;
        strictDeps = true;

        nativeBuildInputs = [
          pkgs.pkg-config
          llvmPkgs.clang
          pkgs.makeWrapper
        ];

        buildInputs = [
          pkgs.pipewire
          llvmPkgs.libclang
          pkgs.libxkbcommon
          pkgs.wayland
          pkgs.libGL
          pkgs.vulkan-loader
          pkgs.libx11
          pkgs.libxcursor
          pkgs.libxi
          pkgs.libxrandr
          pkgs.libxinerama
          pkgs.libxcb
        ];

        LIBCLANG_PATH = "${llvmPkgs.libclang.lib}/lib";
      };

      # Build only Cargo dependencies first so they are cached and reused.
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      pipemeeter = craneLib.buildPackage (
        commonArgs
        // {
          inherit cargoArtifacts;
          pname = "pipemeeter";
          version = "0.1.0";

          postFixup = ''
            wrapProgram "$out/bin/pipemeeter" \
              --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeLibs}"
          '';
        }
      );
    in {
      packages.default = pipemeeter;

      apps.default = {
        type = "app";
        program = "${pipemeeter}/bin/pipemeeter";
      };

      devShells.default = pkgs.mkShell {
        packages = [
          pkgs.rustc
          pkgs.cargo
          pkgs.rustfmt
          pkgs.clippy
          pkgs.rustPlatform.rustLibSrc
          pkgs.pkg-config
          llvmPkgs.clang
          llvmPkgs.libclang
          pkgs.pipewire
          pkgs.libxkbcommon
          pkgs.wayland
          pkgs.libGL
          pkgs.vulkan-loader
          pkgs.libx11
          pkgs.libxcursor
          pkgs.libxi
          pkgs.libxrandr
          pkgs.libxinerama
          pkgs.libxcb
        ];

        LIBCLANG_PATH = "${llvmPkgs.libclang.lib}/lib";
        LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath runtimeLibs}";
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    });
}
