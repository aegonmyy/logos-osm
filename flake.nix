{
  description = "OpenStreetMap distribution on Logos — Geofabrik hosting, Logos Storage mirrors, LEZ region registry, and a Basecamp app";

  inputs = {
    logos-module-builder = {
      url = "github:logos-co/logos-module-builder";
      # Override the builder's bundler with our fork: it adds ktmw32 to
      # the Windows system-DLL allowlist (RocksDB's KTM import) on top of
      # upstream main, which already allowlists winscard/bcryptprimitives
      # — our pinned lock predated those. Remove once upstream merges
      # ktmw32.
      inputs.nix-bundle-lgx.url = "github:aegonmyy/nix-bundle-lgx/main";
    };
    nixpkgs.follows = "logos-module-builder/nixpkgs";
  };

  outputs = inputs@{ self, nixpkgs, logos-module-builder, ... }:
    let
      lib = nixpkgs.lib;
      supportedSystems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" "x86_64-windows" ];

      # The OSM core module. Its Rust core (liblogos_osm.so on Linux,
      # .dylib on macOS, .dll on Windows) is loaded at runtime via dlopen.
      # Linux: staged from module/vendor/x86_64-linux/ via external_libraries.
      # Windows: staged from module/vendor/x86_64-windows/ via postInstall.
      # macOS: the cdylib is built from source by the CI runner (native
      # macOS) and staged via postInstall. The builder evaluates
      # per-system, so the darwin postInstall only runs on darwin runners.
      osmModule = logos-module-builder.lib.mkLogosModule {
        src = ./module;
        configFile = ./module/metadata.json;
        flakeInputs = inputs;
        postInstall = ''
          # Windows: vendored DLL.
          if ls "$out"/lib/*.dll >/dev/null 2>&1 && [ -f "${toString ./module}/vendor/x86_64-windows/logos_osm.dll" ]; then
            cp -L "${toString ./module}/vendor/x86_64-windows/logos_osm.dll" "$out"/lib/
            echo "vendored windows core (logos_osm.dll)"
          fi
        '';
      };

      # The Basecamp app (QML): host/registrar and consumer flows. Talks to
      # the OSM module over RemoteObjects.
      osmAppModule = logos-module-builder.lib.mkLogosQmlModule {
        src = ./app;
        configFile = ./app/metadata.json;
        flakeInputs = inputs // { osm = osmModule; };
      };

      # The minimal embedding example: a Basecamp module that depends only on
      # the SDK module and uses the consumer subset of its API to resolve a
      # region to the CID the registry records for it. This is the shape the
      # prize asks other modules to copy.
      osmConsumerModule = logos-module-builder.lib.mkLogosQmlModule {
        src = ./examples/consumer-app;
        configFile = ./examples/consumer-app/metadata.json;
        flakeInputs = inputs // { osm = osmModule; };
      };

      # Build the OSM Rust cdylib from source for darwin. The osm-sdk crate
      # compiles as a cdylib producing liblogos_osm.dylib. Only evaluated
      # on darwin systems (the packagesFor closure for each system only
      # references this when system is darwin).
      osmCdylibFor = system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in pkgs.rustPlatform.buildRustPackage {
          pname = "logos-osm-cdylib";
          version = "1.5.0";
          src = ./.;
          sourceRoot = "source/osm-sdk";
          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };
          buildInputs = [ pkgs.openssl pkgs.pkg-config ];
          env.RISC0_DEV_MODE = "1";
          doCheck = false;
          installPhase = ''
            runHook preInstall
            mkdir -p $out/lib
            find target/release -maxdepth 1 -name 'liblogos_osm.*' -type f \
              -exec cp {} $out/lib/ \;
            runHook postInstall
          '';
        };

      packagesFor = system:
        let
          # On darwin, overlay the cdylib into the osm module's lib dir.
          # We do this by overriding the module's postInstall to also copy
          # the cdylib. The cdylib derivation is only referenced (and thus
          # only built) when this system closure is evaluated, so Linux and
          # Windows evaluations never touch it.
          isDarwin = lib.hasSuffix "darwin" system;
          osmModule' =
            if isDarwin
            then logos-module-builder.lib.mkLogosModule {
              src = ./module;
              configFile = ./module/metadata.json;
              flakeInputs = inputs;
              postInstall = ''
                # Stage the cdylib built from source for darwin.
                cdylib="${osmCdylibFor system}/lib/liblogos_osm.dylib"
                if [ -f "$cdylib" ]; then
                  mkdir -p "$out"/lib
                  cp -L "$cdylib" "$out"/lib/liblogos_osm.dylib
                  echo "staged darwin core (liblogos_osm.dylib)"
                fi
              '';
            }
            else osmModule;
          osmPkgs = osmModule'.packages.${system} or {};
          appPkgs = osmAppModule.packages.${system} or {};
          consumerPkgs = osmConsumerModule.packages.${system} or {};
          prefix = tag: set:
            lib.mapAttrs' (k: v: lib.nameValuePair "${tag}-${k}" v) set;
        in
          osmPkgs
          // prefix "osm" osmPkgs
          // prefix "osm-app" appPkgs
          // prefix "osm-consumer" consumerPkgs;
    in {
      packages = lib.genAttrs supportedSystems packagesFor;
    };
}
