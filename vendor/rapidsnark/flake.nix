{
  description = "Rapidsnark Environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";

  outputs =
    { nixpkgs, ... }:
    let
      lib = nixpkgs.lib;

      rapidsnarkVersion = builtins.head
        (lib.splitString "\n" (builtins.readFile ./RAPIDSNARK_VERSION));

      hashes = (builtins.fromJSON (builtins.readFile ./nix-hashes.json)).rapidsnark.${rapidsnarkVersion};

      forkBase  = "https://github.com/logos-blockchain/logos-blockchain-rust-rapidsnark/releases/download/rapidsnark-pic-v${rapidsnarkVersion}";
      iden3Base = "https://github.com/iden3/rapidsnark/releases/download/v${rapidsnarkVersion}";

      # Linux PIC archives are built by us (see .github/workflows/build-pic-archives.yml);
      # macOS archives are taken from the upstream iden3 release.
      assetUrl = system: {
        "x86_64-linux"   = "${forkBase}/rapidsnark-linux-x86_64-pic-v${rapidsnarkVersion}.zip";
        "aarch64-linux"  = "${forkBase}/rapidsnark-linux-aarch64-pic-v${rapidsnarkVersion}.zip";
        "aarch64-darwin" = "${iden3Base}/rapidsnark-macOS-arm64-v${rapidsnarkVersion}.zip";
        "x86_64-darwin"  = "${iden3Base}/rapidsnark-macOS-x86_64-v${rapidsnarkVersion}.zip";
      }.${system};

      systems = builtins.attrNames hashes;
      forAll  = lib.genAttrs systems;

      mkRapidsnark = system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        pkgs.stdenv.mkDerivation {
          pname   = "rapidsnark";
          version = rapidsnarkVersion;
          phases  = [ "installPhase" ];

          # fetchzip extracts and strips the top-level directory; src points directly to the content.
          src = pkgs.fetchzip {
            url  = assetUrl system;
            hash = hashes.${system};
          };

          # Exposed archives are static. Linux ones are built with -fPIC (see build-pic-archives.yml); remaining are upstream iden3.
          installPhase = ''
            mkdir -p $out
            cp $src/lib/* $out/
          '';

          meta.platforms = builtins.attrNames hashes;

          passthru.version = rapidsnarkVersion;
        };
    in
    {
      packages = forAll (system: {
        rapidsnark = mkRapidsnark system;
        default    = mkRapidsnark system;
      });
    };
}
