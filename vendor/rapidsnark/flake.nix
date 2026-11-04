{
  description = "Logos Blockchain Rust Rapidsnark";

  # The rapidsnark fork builds its Windows libraries with logos-nix's MinGW, so they must be linked with it too.
  # Keep this pinned to the same logos-nix as the rapidsnark fork.
  inputs.logos-nix.url = "github:logos-co/logos-nix";

  outputs =
    { logos-nix, ... }:
    let
      buildPkgs = logos-nix.legacyPackages.x86_64-linux;
      windowsPkgs = buildPkgs.pkgsWindows;
      cc = windowsPkgs.stdenv.cc;
    in
    {
      # Cross-builds for x86_64-pc-windows-gnu, e.g. `cargo build --target x86_64-pc-windows-gnu`.
      devShells.x86_64-linux.windows-cross = buildPkgs.mkShell {
        packages = [
          cc
          # rust-witness builds w2c2 with cmake for the tests.
          buildPkgs.cmake
        ];

        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${cc.targetPrefix}gcc";
        CC_x86_64_pc_windows_gnu = "${cc.targetPrefix}gcc";
        CXX_x86_64_pc_windows_gnu = "${cc.targetPrefix}g++";
        AR_x86_64_pc_windows_gnu = "${cc.targetPrefix}ar";
        # Rust expects pthreads, but logos-nix's MinGW uses mcfgthread.
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = builtins.concatStringsSep " " [
          "-L native=${windowsPkgs.windows.pthreads}/lib"
          "-C link-arg=-l:libmcfgthread.a"
          "-C link-arg=-lntdll"
          "-C link-arg=-lkernel32"
        ];
      };
    };
}
