{
  description = "OpenStreetMap distribution on Logos — Geofabrik hosting, Logos Storage mirrors, LEZ region registry, and a Basecamp app";

  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";
    nixpkgs.follows = "logos-module-builder/nixpkgs";
  };

  outputs = inputs@{ self, nixpkgs, logos-module-builder, ... }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];

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
          prefix "osm" osmPkgs // prefix "osm-app" appPkgs;
    in {
      packages = nixpkgs.lib.genAttrs supportedSystems packagesFor;
    };
}
