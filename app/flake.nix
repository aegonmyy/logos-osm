{
  description = "OSM distribution on Logos: the Basecamp app (host/registrar and consumer flows)";

  # The catalog release action publishes one module per workflow run: it cds
  # into `module_path` and builds `.#lgx-portable` from the flake it finds
  # there. So the app needs its own flake root to be published alongside the
  # SDK module, which is why this lives in `app/` rather than only in the
  # repository-root flake.

  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";
    nixpkgs.follows = "logos-module-builder/nixpkgs";
    # The SDK module this app depends on (see metadata.json "dependencies").
    # Pinned to the repository's default branch; bump to a tag once the port
    # is merged.
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
