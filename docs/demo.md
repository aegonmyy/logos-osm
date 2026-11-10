# OSM on Logos — demo script

## Segment 1 — Intro & Connect

**Intro / pleasantries**

Alright — thanks for having me. What we're going to walk through is
**OpenStreetMap data distribution on Logos**: how map data gets hosted,
verified, and consumed — all wired through a Basecamp app, with the heavy
lifting on-chain where it belongs.

The flow is four parts: **Connect** (the plumbing), **Fetch** (hosting
data), **Regions** (what's hosted and what's on-chain), and **Consumer**
(pulling data back down and using it). Let's start at the top — Connect.

**Connect**

Everything begins here, because every operation downstream depends on
these four settings.

- **Storage URL** — this points to where the **Codex storage node** is
  stood up. That's the thing that actually *stores the files* — the map
  blobs, addressed by content ID. The little dot next to it goes green
  the moment the node answers, so you always know it's reachable.
- **Geofabrik URL** — the **source of the OSM data**. Geofabrik publishes
  the regional OpenStreetMap extracts, so when we fetch a region, the raw
  download comes from here.
- **Cache dir** — where that data is **stored locally** on disk once it's
  down. Each machine keeps its own cache and its own catalog of what it
  has hosted or imported.
- **Registrar** — the **account tied to whatever transactions you do**.
  Specifically, the wallet account that signs any on-chain registration.
  The app never touches private keys — it builds the transaction; *your
  wallet* signs it.

So: where to store, where to download from, where to keep it, and who
signs. Four fields, and once they're set, the rest of the app knows
exactly what to do.

## Segment 2 — Fetch

**Fetch**

So here's **Fetch** — the way you actually get data *from* Geofabrik
*onto* Logos.

First you select the region you want to host, from the dropdown — and you
can pick as many as you like, up to 25 at once. Then you hit **Fetch**.
Down in the task panel you watch it go: `downloading 45 / 80 MB …` as it
pulls each region from Geofabrik.

For our use case, we'll host **china/henan**. *(host it)*

Now — this makes the files available on **your Codex storage**,
addressed by a content ID. But it does *not* make them discoverable by
anyone. The blob exists on the node; the chain doesn't know about it
yet. For it to be discoverable, you have to **register it on-chain**.

At this stage we haven't wired registration natively into the app — so
you do that from the CLI. After the fetch, this **"Finish on-chain"**
card pops up with the exact command:

```
osm register china/henan --registrar <your-account> --sign
```

*(show the CLI, submit the tx, watch it get included)*

So the split is clean: **Fetch** puts the bytes on storage; **register**
puts the record on-chain. One makes it exist, the other makes it
findable.

## Segment 3 — Regions

**Regions**

Alright, **Regions**. This is where you see the **overall state of
discoverable data on Logos**.

The table lists the region set, and each row carries a badge:
**registered** (a yellow dot — it's on-chain, anyone can find it) or
**not registered** (it's hosted on storage but the chain doesn't know
about it yet). Up top you see the count — "28 in the set · 3
registered" — and there's a filter to slice it: **All · Registered ·
Not registered**. Click **Registered** and you see just the ones the
chain knows about.

Click any row and it reads the **full registry entry straight from the
chain** — the content ID, the checksum, the version, the source URL.
That's the provenance: not "this file exists somewhere", but "the chain
attests these exact bytes, signed by this registrar".

And **Re-check** re-reads the whole registry in one bulk call — so
right after you register something from the CLI, you hit Re-check and
watch the badge flip to registered, live.

The thing worth stressing here: this is **not your local catalog**. The
registry is global — anyone running this app, anywhere, reads the *same*
chain, sees the *same* registered set. So Regions is the trust layer:
it's what makes a hosted blob *discoverable and verifiable* by the whole
network, not just sitting on your storage node. Local catalog says "I
have it"; the registry says "the network knows it's there, and whose it
is".

One honest caveat, though: registration guarantees the record is
on-chain — it does **not** guarantee you can fetch the file right now.
The blob could be on a Codex node that's down, or gone. So in this
version, a registered badge means "it was registered, the chain attests
the bytes" — not "those bytes are available to download at this very
moment". Registered ≠ available; that's a gap we know about and will
close.

## Segment 4 — Consumer

**Consumer**

Alright, this is the final section — **Consumer**. Three features, all
about getting map data *usable* on your end.

**§01 Restore a registered snapshot.** This lets you pull map data
directly from Logos Storage, served by the peers hosting it. Now — not
all regions are hosted or registered, so the first thing is a quick look
through the **Regions** tab: filter to **Registered** and you see what's
actually out there on-chain. Click the one you want and its full registry
entry prints — the **CID** (the location of the file on storage) and the
**MD5** (the checksum of the file *as it was at registration*). That's
what you restore against. The app downloads the blob by CID, computes the
checksum, and **only if that computed checksum equals the advertised one
is it kept**. A mismatch means the bytes changed since registration — and
it's rejected. So restore is integrity-guaranteed: you get exactly what
was registered, or you get nothing.

**§02 Import a local file.** Suppose you have a local `.osm.pbf` that you
want hosted. There are two paths here. Plain **Import (validate +
catalog)** just validates it's a real PBF and records it in your local
catalog — no network, accepts whatever you give it. The full **Verify +
store + register tx** path is stricter: it checks the file against
Geofabrik's *currently-published* checksum, and **outdated ones are
rejected** — "refusing to store/register a file that no longer matches
upstream". Which is honestly a bit harsh right now: we reject any file
that isn't the absolute latest release. We intend to solve that with
**versions** in the future, so an older snapshot can still be hosted and
registered as a known version rather than rejected outright.

**§03 Update check.** Finally, this checks Geofabrik for any newer
version of the selected region — compares your local version against the
latest published, and tells you `up to date` or `update available`. Of
course it's a shame that, downstream, we just assume previous versions
are illegal — which is the same versions problem, and we intend to fix it
in the future too.

## Closing

So that's the whole loop: you **Fetch** data onto Logos storage, you
**register** it on-chain to make it discoverable, you watch the **Regions**
tab reflect the global state of what's registered and whose it is, and
you **consume** it back down — integrity-guaranteed, straight from the
peers serving it. Host, register, discover, consume. Map data on Logos —
verifiable, and shared by the whole network. Thanks.
