# Ship runbook — logos-osm releases

Follow this exactly. Every gotcha below is one I hit for real. Don't improvise.

## Architecture (read first)

Three repos, three roles. Do not confuse them:

| Repo | Role | Actions? |
|---|---|---|
| `aegonmyy/logos-osm` | **Source** (code, metadata, CI) | billing-gated (CI only; resets monthly) |
| `ameenme/logos-modules-release-base` | **Build box** (nix builds run here) | yes, free |
| `aegonmyy/logos-modules-release-base` | **Catalog** (what Basecamp fetches) | none needed — releases served via API |

Builds happen on **ameenme** (its `submodules/logos-osm` points at `aegonmyy/logos-osm`).
The built releases are then **mirrored** to **aegonmyy**, which is the canonical catalog
Basecamp reads. Never dispatch release workflows on aegonmyy — they won't run (billing gate).

## Tokens (never persist in remotes; one-shot push URLs)

- aegonmyy: `~/.secrets/aegonmyy-token.env` → `ghp_…` (scopes: repo, workflow)
- ameenme:  `~/.secrets/ameenme-codespace-token.env` → `ghp_…` (scopes: repo, workflow)

```sh
AE=$(grep -o 'ghp_[A-Za-z0-9]*' ~/.secrets/aegonmyy-token.env | head -1)
AM=$(grep -o 'ghp_[A-Za-z0-9]*' ~/.secrets/ameenme-codespace-token.env | head -1)
```

## Versioning (SemVer — locked in STATE.md)

`MAJOR.MINOR.PATCH`: **MINOR** for features, **PATCH** for fixes, **MAJOR** for breaking.
Bump **all three** metadata files together: `metadata.json` (root), `module/metadata.json`,
`app/metadata.json`. The release workflow reads the **root** one.

## The ship procedure

### 1. Finish + verify the code

```sh
cd /workspaces/workspace/logos-osm
cargo fmt
cargo check -p logos-osm -p logos-osm-cli -p osm-integration-tests --examples
cargo test -p logos-osm --lib        # expect 42 passed, 1 ignored
```

CI enforces `cargo fmt` — an unformatted tree fails the "Build + unit tests" job. Always `cargo fmt` first.

### 2. Bump the version (ALL THREE files) — and STAGE them

```sh
OLD=1.4.0; NEW=1.5.0   # set these
for f in metadata.json module/metadata.json app/metadata.json; do
  sed -i "s/\"version\": *\"$OLD\"/\"version\": \"$NEW\"/" "$f"
done
grep -H '"version"' metadata.json module/metadata.json app/metadata.json   # sanity check
git add metadata.json module/metadata.json app/metadata.json               # CRITICAL — see gotcha #1
```

### 3. Commit + push to aegonmyy

Stage only the files you intend to ship. **Exclude** internal docs (`TASK.md`,
`docs/demo.md`, `task_90.md`) — they contain VPS coords or personal scripts.

```sh
git add <your files>   # plus the three metadata files from step 2
git commit -m "v$NEW: <summary>"
# NO "Co-Authored-By" line in logos-osm commits (repo rule).
git push "https://$AE@github.com/aegonmyy/logos-osm.git" HEAD:main
NEWSHA=$(git rev-parse HEAD); echo "new sha: $NEWSHA"
```

### 4. Bump the submodule in the ameenme release-base + push

```sh
cd /workspaces/workspace/osm-release-base/submodules/logos-osm
git fetch -q origin main && git checkout -q "$NEWSHA"
cd /workspaces/workspace/osm-release-base
git add submodules/logos-osm
git commit -m "logos-osm: v$NEW ($NEWSHA)"
git push "https://$AM@github.com/ameenme/logos-modules-release-base.git" HEAD:main
```

### 5. Dispatch the release workflow(s) on ameenme

```sh
REPO=ameenme/logos-modules-release-base
# Dispatch the ones that changed. Usually osm + osm_app. Add osm-consumer if the
# consumer-app changed.
for wf in release-logos-osm.yml release-logos-osm-app.yml; do
  curl -s -o /dev/null -w "$wf: %{http_code}\n" -X POST \
    -H "Authorization: token $AM" -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/$REPO/actions/workflows/$wf/dispatches" \
    -d '{"ref":"main"}'
done
```

**Re-releasing the same version** (replacing an existing tag, e.g. a build fix at the
same version): add `"inputs":{"force_build":"true"}`:
```sh
-d '{"ref":"main","inputs":{"force_build":"true"}}'
```

A newly-added workflow file can take ~10–20s to register after push. If the dispatch
returns **404**, wait and retry — the workflow exists, GitHub just hasn't indexed it.

### 6. Monitor the builds (~15 min, 4 platforms each)

```sh
cutoff=$(date -u -d '1 minute ago' +%Y-%m-%dT%H:%M:%SZ)
while true; do
  curl -s --max-time 15 -H "Authorization: token $AM" \
    "https://api.github.com/repos/ameenme/logos-modules-release-base/actions/runs?per_page=15" |
  python3 -c "
import json,sys
cutoff='$cutoff'
d=json.load(sys.stdin)
for r in [r for r in d.get('workflow_runs',[]) if r['name'].startswith('Release logos-osm') and r['created_at']>cutoff][:2]:
    print(f\"{r['name']}: {r['status']}/{r['conclusion'] or '-'}\")
"
  sleep 45
  # break when both show completed/...
done
```

**Coverage check:** a monitor that greps only for success is silent through a crash.
Match terminal conclusions: `completed/success` AND `completed/failure`.

### 7. Rebuild the index (often needs a manual kick)

When two releases publish near-simultaneously, two `rebuild-index` runs fire and one
gets **cancelled** → the index misses a package. After the releases land, dispatch it
manually and verify every package appears:

```sh
curl -s -o /dev/null -w "%{http_code}\n" -X POST \
  -H "Authorization: token $AM" -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/ameenme/logos-modules-release-base/actions/workflows/rebuild-index.yml/dispatches" \
  -d '{"ref":"main"}'
# verify (ameenme):
curl -sL "https://github.com/ameenme/logos-modules-release-base/releases/download/index/index.json?cb=$RANDOM" |
  python3 -c "import json,sys;d=json.load(sys.stdin);print([(p['name'],p['versions'][0]['manifest']['version']) for p in d['packages']])"
```

Expect `osm`, `osm_app`, `osm_consumer` all present at their bumped versions.

## Mirror ameenme → aegonmyy (the catalog Basecamp reads)

After ameenme's releases + index are correct, mirror them to aegonmyy. Releases are
served via API (no Actions needed). This script copies any missing release assets and
rewrites the index URLs from `ameenme` → `aegonmyy`:

```sh
cat > /tmp/mirror_catalog.py <<'PY'
import json, urllib.request
AM = open('/dev/stdin').read().strip() if False else __import__('os').environ['AM']
AE = __import__('os').environ['AE']
SRC = "ameenme/logos-modules-release-base"; DST = "aegonmyy/logos-modules-release-base"
def api(url, tok, data=None, method=None, ctype="application/json", raw=False):
    h = {"Authorization": f"token {tok}", "Accept": "application/vnd.github+json", "Content-Type": ctype}
    if raw: h["Accept"] = "application/octet-stream"
    req = urllib.request.Request(url, data=data, headers=h, method=method)
    with urllib.request.urlopen(req) as r: return r.status, r.read()
def japi(url, tok, payload=None, method=None):
    data = json.dumps(payload).encode() if payload is not None else None
    return json.loads(api(url, tok, data=data, method=method)[1] or b'{}')
_, dst_raw = api(f"https://api.github.com/repos/{DST}/releases?per_page=100", AE)
dst_tags = {r["tag_name"] for r in json.loads(dst_raw)}
dst_index = next((r for r in json.loads(dst_raw) if r["tag_name"]=="index"), None)
_, src_raw = api(f"https://api.github.com/repos/{SRC}/releases?per_page=100", AM)
mirrored = skipped = 0
for rel in json.loads(src_raw):
    tag = rel["tag_name"]
    if tag == "index": continue
    if tag in dst_tags: skipped += 1; continue
    nr = japi(f"https://api.github.com/repos/{DST}/releases", AE,
              {"tag_name": tag, "name": rel["name"], "body": rel.get("body") or ""})
    up = nr["upload_url"].replace("{?name,label}", "")
    for a in rel["assets"]:
        _, content = api(a["url"], AM, raw=True)
        api(f"{up}?name={a['name']}", AE, data=content, method="POST", ctype="application/octet-stream")
    mirrored += 1
si = next(r for r in json.loads(src_raw) if r["tag_name"]=="index")
sia = next(a for a in si["assets"] if a["name"]=="index.json")
idx = api(sia["url"], AM, raw=True)[1].replace(b"ameenme", b"aegonmyy")
old = next((a for a in dst_index["assets"] if a["name"]=="index.json"), None)
if old: api(old["url"], AE, method="DELETE")
up = dst_index["upload_url"].replace("{?name,label}", "")
api(f"{up}?name=index.json", AE, data=idx, method="POST", ctype="application/octet-stream")
print(f"mirrored={mirrored} skipped={skipped} index rewritten ({len(idx)} bytes)")
PY
AM=$AM AE=$AE python3 /tmp/mirror_catalog.py
```

### After mirroring: the index CDN 504s for ~3 min (gotcha #3)

Replacing the `index.json` asset leaves GitHub's CDN with a stale redirect that returns
**504** until the cache TTL clears (~3 min). Do NOT re-replace repeatedly — that spawns
more dead blobs. Just wait + poll:

```sh
URL="https://github.com/aegonmyy/logos-modules-release-base/releases/download/index/index.json"
for i in $(seq 1 20); do
  out=$(curl -sL "$URL?cb=$i" 2>/dev/null | python3 -c "
import json,sys
try: d=json.load(sys.stdin)
except: sys.exit(0)
print(','.join(f\"{p['name']}={p['versions'][0]['manifest']['version']}\" for p in d['packages']))
" 2>/dev/null)
  echo "$out" | grep -q osm_consumer && { echo "LIVE: $out"; break; }
  sleep 20
done
```

## Verify (end-to-end, no auth)

```sh
# 1. catalog descriptor self-points at aegonmyy
curl -s https://raw.githubusercontent.com/aegonmyy/logos-modules-release-base/main/logos-repo.json | grep indexUrl
# 2. index lists all packages at the right versions
curl -sL "https://github.com/aegonmyy/logos-modules-release-base/releases/download/index/index.json?cb=$RANDOM" | python3 -m json.tool | head
# 3. a .lgx resolves publicly (302 → 200)
curl -sLI "https://github.com/aegonmyy/logos-modules-release-base/releases/download/osm-v$NEW/osm-$NEW.lgx" | grep -iE "^HTTP"
```

All three must be 200. Use `raw.githubusercontent.com` (canonical) — the
`github.com/…/raw/…` redirect wrapper 504s intermittently (gotcha #6).

## CI on aegonmyy (separate from releases)

`ci.yml` is **workflow_dispatch only** — it does NOT auto-run on push. After a source
push, dispatch it manually to verify:

```sh
curl -s -o /dev/null -w "%{http_code}\n" -X POST \
  -H "Authorization: token $AE" -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/aegonmyy/logos-osm/actions/workflows/ci.yml/dispatches" \
  -d '{"ref":"main"}'
```

5 jobs should pass: Build+unit tests, Live LEZ e2e, CU cycle profile, macOS run,
`.lgx bundles build + are callable`. If the dispatch 403s or fails to start, the
aegonmyy Actions spending gate is active — it resets monthly; wait or run CI elsewhere.
The `Build Windows cdylib` workflow is separate (manual dispatch) and does not affect
the CI badge; the Windows `.dll` is vendored at `module/vendor/x86_64-windows/logos_osm.dll`.

## Gotchas (the full list — each cost me a round-trip)

1. **Silent skip on version match.** If the root `metadata.json` version equals an
   existing release tag, the workflow exits `success` and publishes **nothing**. Always
   bump the version AND `git add` the metadata files (I once bumped with `sed` but forgot
   to stage → the commit didn't include the bump → silent skip).
2. **rebuild-index cancellation.** Two releases publishing at once → two index rebuilds,
   one cancelled → index misses a package. Dispatch `rebuild-index.yml` manually and
   verify all packages present before mirroring.
3. **Index CDN 504 after replace.** Mirroring rewrites aegonmyy's `index.json` by
   delete+upload; the public URL 504s ~3 min. Wait it out (poll). Don't re-replace.
4. **cache.nix.logos.co flake.** The nix binary cache is often unreachable; builds fall
   back to `cache.nixos.org` (slower, works). Not fatal. If a Windows leg fails on it,
   retry the dispatch with `force_build`.
5. **New workflow 404 on dispatch.** A freshly-pushed workflow file takes ~10–20s to
   register. A 404 dispatch right after push just means "wait + retry."
6. **github.com/…/raw/ → 504.** Use `raw.githubusercontent.com` (canonical) for catalog
   links; the `github.com/…/raw/…` redirect wrapper intermittently 504s. README + the
   submission doc must use the canonical form.
7. **No Co-Authored-By in logos-osm commits.** Repo rule. (The release-base submodule
   bump commits follow the same convention.)
8. **ameenme builds, aegonmyy mirrors.** Don't dispatch release workflows on aegonmyy
   (billing-gated). Don't expect ameenme to be the public catalog — it's the build box;
   aegonmyy is what Basecamp reads.
9. **`cargo fmt` before push.** CI's "Build + unit tests" job runs a format check that
   fails the job on any diff. Run `cargo fmt` and stage the reformatted files.
10. **Exclude internal docs.** `TASK.md`, `docs/demo.md`, `task_90.md` stay out of
    commits (VPS coords / personal scripts / task specs). Stage files explicitly, never
    `git add -A`.

## Quick reference — one-shot ship (everything already verified)

```sh
cd /workspaces/workspace/logos-osm
AE=$(grep -o 'ghp_[A-Za-z0-9]*' ~/.secrets/aegonmyy-token.env | head -1)
AM=$(grep -o 'ghp_[A-Za-z0-9]*' ~/.secrets/ameenme-codespace-token.env | head -1)
# bump (ALL THREE), fmt, commit, push
cargo fmt && git add -u && git add metadata.json module/metadata.json app/metadata.json
git commit -m "vX.Y.Z: …" && git push "https://$AE@github.com/aegonmyy/logos-osm.git" HEAD:main
NEWSHA=$(git rev-parse HEAD)
# submodule + dispatch
cd /workspaces/workspace/osm-release-base/submodules/logos-osm && git fetch -q origin main && git checkout -q $NEWSHA
cd /workspaces/workspace/osm-release-base && git add submodules/logos-osm && git commit -q -m "logos-osm: vX.Y.Z ($NEWSHA)"
git push "https://$AM@github.com/ameenme/logos-modules-release-base.git" HEAD:main
for wf in release-logos-osm.yml release-logos-osm-app.yml; do
  curl -s -o /dev/null -w "$wf: %{http_code}\n" -X POST -H "Authorization: token $AM" \
    -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/ameenme/logos-modules-release-base/actions/workflows/$wf/dispatches" \
    -d '{"ref":"main"}'
done
# then: monitor (step 6) → rebuild-index (step 7) → mirror (above) → verify
```
