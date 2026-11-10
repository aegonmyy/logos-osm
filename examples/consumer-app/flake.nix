{
  description = "OSM on Logos: minimal consumer embedding example — resolves a region to its CID; hosts nothing.";

  # The catalog release action publishes one module per workflow run: it cds
  # into `module_path` and builds `.#lgx-portable` from the flake it finds
  # there (see app/flake.nix for the same pattern). So the consumer app needs
  # its own flake root to be published as a standalone module alongside the
  # SDK, rather than only being defined in the repository-root flake.
  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";
    nixpkgs.follows = "logos-module-builder/nixpkgs";
    # The SDK module this app depends on (metadata.json "dependencies": ["osm"]).
    logos-osm.url = "github:aegonmyy/logos-osm";
    logos-osm.inputs.logos-module-builder.follows = "logos-module-builder";
  };

  outputs = inputs@{ logos-module-builder, logos-osm, ... }:
    logos-module-builder.lib.mkLogosQmlModule {
      src = ./.;
      configFile = ./metadata.json;
      # `osm` is the name metadata.json declares; the builder resolves the
      # dependency's interface from this input.
      flakeInputs = inputs // { osm = logos-osm; };
    };
}
