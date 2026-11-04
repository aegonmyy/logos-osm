# Contributing

## Fork maintenance

This repository is a fork of [zkmopro/rust-rapidsnark](https://github.com/zkmopro/rust-rapidsnark).

### Remotes

```bash
git remote add upstream https://github.com/zkmopro/rust-rapidsnark.git
```

-   `origin`: this repository.
-   `upstream`: zkmopro/rust-rapidsnark.

### Branches

-   `logos/v<x.y.z>`: our changes on top of upstream's `v<x.y.z>` tag, one branch per upstream version.
    The newest one is the default branch, and all work targets it.
-   `zkmopro/main`: an exact copy of upstream's `main`. Only ever fast-forwarded, never committed to.

`logos/*` branches are never rewritten, so their tags stay in their history.
PRs are squash-merged, so each one is a single commit that is easy to rebase onto a new upstream version.

### Syncing with upstream

1. Fast-forward `zkmopro/main` to `upstream/main` and push it.

   > [!NOTE]
   > If the fast-forward fails, something was committed to `zkmopro/main`.
   > Move it to a branch off the newest `logos/*` branch and reset `zkmopro/main` to `upstream/main`.

2. When upstream tags a new version, e.g. `v0.1.5`, branch off the newest `logos/*` branch and rebase it onto the tag:

   ```bash
   git fetch origin
   git fetch upstream --tags
   git switch -c logos/v0.1.5 origin/logos/v0.1.4
   git rebase --onto v0.1.5 v0.1.4
   git push origin logos/v0.1.5
   ```

3. Make `logos/v0.1.5` the default branch. `logos/v0.1.4` stays as it is.
4. Release it, then update the downstream pins, e.g. the Logos Blockchain node's `Cargo.toml`.

## Rapidsnark

Static desktop builds link the libraries from the [Logos rapidsnark fork](https://github.com/logos-blockchain/logos-blockchain-rapidsnark)'s releases,
which add the platforms and build options that iden3's releases lack.
For details, see its `README.md` and `CONTRIBUTING.md` on the `logos/<RAPIDSNARK_VERSION>` branch.

`crates/download_rapidsnark.sh` pins the version in two lines:

```sh
RAPIDSNARK_VERSION=v0.0.8             # The iden3 version.
LOGOS_RAPIDSNARK_SUFFIX=logos.1-rc.3  # The Logos fork's release of it.
```

Together they are the fork's tag, `v0.0.8-logos.1-rc.3`.

To update, set `LOGOS_RAPIDSNARK_SUFFIX` and the `logos-*` hashes in the script.
If an upstream sync changes `RAPIDSNARK_VERSION`, the fork needs a release for that version first.

## Releasing

Tags are `v<x.y.z>-logos.<n>` on `logos/v<x.y.z>`, e.g. `v0.1.4-logos.1` on `logos/v0.1.4`.
Release candidates add `-rc.<n>`.

Tag the branch's last commit and push the tag:

```bash
git fetch origin
git tag -s -m v0.1.4-logos.1 v0.1.4-logos.1 origin/logos/v0.1.4
git push origin v0.1.4-logos.1
```

Push one tag at a time. `git push --tags` also pushes upstream's tags.
