# Contributing

## Bumping the rapidsnark version

The pinned rapidsnark version is the single source of truth in `RAPIDSNARK_VERSION` at the repo root.

To bump it:

1. **Update the `RAPIDSNARK_VERSION` file**
   Set the new version (without the `v` prefix).
2. **Build new Linux PIC archives**
   Dispatch the `.github/workflows/build-pic-archives.yml` workflow.
3. **Update `nix-hashes.json`**
   Add a new entry for the version. For each platform, grab the zip's SHA256 from the GitHub release and convert it to SRI format:
   ```sh
   nix hash convert --hash-algo sha256 --from base16 --to sri <sha256>
   ```
4. **Update flake consumers**
   In consuming repos, update the flake input's `ref` in `flake.nix` (if pinned to a specific rev) and run `nix flake update <input-name>` to update the lock file.
