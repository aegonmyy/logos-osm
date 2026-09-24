{
  description = "OpenStreetMap distribution on Logos — Geofabrik hosting, Logos Storage mirrors, LEZ region registry, and a Basecamp app";

  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";
    nixpkgs.follows = "logos-module-builder/nixpkgs";
  };

  outputs = inputs@{ self, nixpkgs, logos-module-builder, ... }:
    let
      # Match the module-builder's own system set (lib/common.nix `systems`)
      # so the catalog release action's darwin-arm64 leg has a package to
      # build. x86_64-windows is a cross pseudo-system the builder realises
      # only when logos-nix is an input; it is intentionally absent here, so
      # the windows leg fails that variant alone (the action tolerates it).
      supportedSystems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];

      # The OSM core module. Its Rust core (liblogos_osm.so) is built with
      # cargo and loaded at runtime via LOGOS_OSM_FFI_PATH. It depends on the
      # LEZ wallet/lee crates (pinned in the workspace Cargo.toml), so it
      # builds against Qt + the Logos Core SDK.
      osmModule = logos-module-builder.lib.mkLogosModule {
        src = ./module;
        configFile = ./module/metadata.json;
        flakeInputs = inputs;
      };

      # The Basecamp app (QML): host/registrar and consumer flows. Talks to
      # the OSM module over RemoteObjects.
      osmAppModule = logos-module-builder.lib.mkLogosQmlModule {
        src = ./app;
        configFile = ./app/metadata.json;
        flakeInputs = inputs // { osm = osmModule; };
      };

      packagesFor = system:
        let
          osmPkgs = osmModule.packages.${system} or {};
          appPkgs = osmAppModule.packages.${system} or {};
          prefix = tag: set:
            nixpkgs.lib.mapAttrs' (k: v: nixpkgs.lib.nameValuePair "${tag}-${k}" v) set;
        in
          # Expose the core module's attributes UNDER THEIR BARE NAMES
          # (lgx-portable, install, lgx, default) so the catalog release
          # action's `nix build .#lgx-portable` resolves. The root
          # metadata.json names this module "osm", and the action builds the
          # attribute matching that name's builder output. Prefixing every
          # output as osm-* (the old shape) hid lgx-portable behind
          # osm-lgx-portable and broke all four build legs.
          osmPkgs
          // prefix "osm" osmPkgs
          // prefix "osm-app" appPkgs;
    in {
      packages = nixpkgs.lib.genAttrs supportedSystems packagesFor;
    };
}
