# vc-rs — a modernized Rust runtime for GTA: Vice City (world, player, AI)

**Goal.** Reuse Vice City's assets 1:1 and rebuild the engine around them in Rust on **Bevy + wgpu**,
targeting **modern hardware** — D3D12/Vulkan-class through wgpu, one WGSL/WESL shader path, no D3D8.
This is a *modernization*, not an emulator: the same city, the same art, the same handling data and
ped stats, but without the 2002 bottlenecks (30 fps-coupled logic, 2,000-unit draw distance,
mipmap-less textures, fixed-function lighting, immediate-mode draws).

**Scope: everything except missions.** The player, the world, vehicles, traffic, ped and police AI,
combat, audio, UI — ported and modernized, so the game is genuinely *playable* as a free-roam
sandbox. The compiled mission script (`data/main.scm`) is out of scope.

**Status of this document.** Reconnaissance in §4 was measured against the install in `gta-vc/` on
2026-10-26; every number is reproducible via §15. External claims (crates, Bevy APIs, prior art) are
marked as such. This revision supersedes the 2026-10-26 first draft, whose target was
period-accurate engine emulation with full mission parity.

---

## 1. Revised decisions

| # | Decision | Change |
|---|---|---|
| D1 | **Everything except missions** (was: full game parity) | The SCM v2 VM, mission content, cutscenes, side jobs and story unlocks are **out**. Engine-side systems are **in**, ported and modernized. |
| D2 | **Offline cook** of the original assets | Unchanged — and now load-bearing: mip generation, BC re-encoding, navmesh generation and LOD/impostor authoring all belong in the cook. |
| D3 | **Modernized renderer on Bevy's PBR stack** (was: RenderWare-faithful custom pipeline) | Reversal of the first draft's "no `bevy_pbr`". Rationale below. |
| D4 | **Clean-room, permissive dependencies only** | Unchanged. Apache/MIT; no `re3`/`reVC` (DMCA-blocked), no GPL crates. |
| D5 | **Modern hardware is the target** — D3D12/Vulkan via wgpu, 64-bit, multithreaded, high refresh rates | The original's limits are treated as bugs to fix, not features to reproduce. |
| D6 | **Framerate independence is a hard requirement** | VC is frame-coupled and needs its ~30 fps limiter; every ported system must behave identically at 30/60/144/240 fps render speeds. Verified by test, not by hope (§14.4). |
| D7 | **`reVC` is a black-box oracle, never a source** (2026-10-26) | Its public fork describes itself as decompiled game code under a non-permissive "MIT with Proof-of-Usage" licence aimed at code assistants. Permitted: build it and observe behaviour. Forbidden: reading, quoting or porting its source. §12, §14.5 |

### Why D3 reversed

The first draft excluded `bevy_pbr` to reproduce RW 3.4 fixed-function shading exactly. That
constraint dissolved with D3's change of intent: we now *want* modern shading. Building custom
materials on Bevy's mesh pipeline (clustered forward lighting, cascaded shadow maps, prepass, TAA,
HDR/tonemapping) buys years of engine work we would otherwise reimplement — while custom `Material`
impls still preserve the RenderWare material semantics that define the look (alpha-test modes, dual
texture, prelit vertex colours, blend/dest-alpha).

Verified context that makes this cheap: **VC ships no shader bytecode at all** (§4.5). The reason
iw4L needed a D3D9-SM3→WGSL translator — D3D9 shader blobs, ~91 KB of Rust — does not exist here.

---

## 2. Scope boundary

**In (engine-side, ported + modernized):**

- World: IMG, DFF, TXD, COL, IPL/IDE, cull zones, LOD, streaming, water, weather/time-of-day.
- Player: on-foot movement, camera, input, model/animation, health/stats, wanted level.
- Vehicles: `handling.cfg` model, car models/COL, wheel dummies, damage, radio stations, traffic AI.
- Peds: population/spawning by zone, task/event AI, stats, groups, relationships, animation.
- Police: wanted escalation, pursuit, roadblocks, on-foot response.
- Combat and world interaction: weapons (`weapon.dat`), damage, particles (`particle.cfg`), object
  physics/breaking (`object.dat`), 2dfx effects (985 entries), explosions.
- Audio, HUD/radar, menus, settings, save/load of free-roam state.

**Out (script-driven content):** missions, cutscenes, side jobs (taxi, vigilante, paramedic,
firefighter), shops/purchases, rampages, phone calls, story unlocks, pickups *as placed by scripts*.

**Replaced by us:** a `vc_director` subsystem that reconstructs the ambient behaviour the scripts
used to provide — spawn density and mix by zone/time, police heat tuning, pickups, ambient events,
weather progression. This is authored behaviour, not ported behaviour, and it is explicitly listed as
such in the fidelity ledger.

**Open boundary question** (§9 R4): in VC, some ambient policy that a modern reader would assume was
engine-side lived in `main.scm`. The probe at M0.5 determines how much of the director must be
authored from scratch.

---

## 3. Fidelity ledger — what stays identical, what modernizes

This is the project's honesty mechanism: a reviewer should be able to see exactly where we diverge.

| Stays 1:1 (byte-identical assets, unchanged data) | Modernized | Ported behaviour (same rules, new implementation) |
|---|---|---|
| All art: 4,617 DFF, 1,368 TXD, COL, IFP, IPL/IDE | Renderer and shading | Player movement (= the original's arcade feel) |
| Map layout, 9,114 instances, 3,789 object defs, 342 time objects, 25,441 path nodes | Draw distance (FarClp 2,000 → 5,000–8,000 + atmosphere) | Vehicle handling (`handling.cfg` values used verbatim) |
| `handling.cfg`, `weapon.dat`, `pedstats.dat`, `pedgrp.dat`, `ped.dat`, `carcols.dat`, `object.dat` | Mip chains + anisotropy + BC re-encode | Ped AI rules (stats, relationships, task outcomes) |
| `timecyc.dat`, `water.dat`, `2dfx` entries | Lighting model, shadows, water shader | Police escalation ladder |
| Audio: `sfx.RAW`/`sfx.SDT`, radio, 1,120 WAV, 90 MP3 | Mixer, spatialization, reverb | Traffic rules and junction behaviour |
| Animation data (`ped.ifp`, `cuts.img`) | Streaming, culling, instancing, frame pacing | Collision response and surface effects |
| | UI/HUD (widescreen, high-DPI, gamepad) | |
| | **Authored, not ported:** `vc_director` (spawn/ambient policy), generated nav mesh (if the original node graph is missing) | |

Rule of thumb we hold ourselves to: **the data is the game; the engine is ours.** If a modernized
system changes observable behaviour, that change is listed in the parity matrix (§14.3), not glossed.

---

## 4. Verified reconnaissance of `gta-vc/`

Install: **GTA: Vice City, PC, version 1.00.000** (Steam layout, `installscript.vdf` registers
`Rockstar Games\Grand Theft Auto Vice City\1.00.000`). 1.5 GB across 1,436 files.

**Engine identity, from strings in `gta-vc.exe`:**

```
@@(#)$Id: //RenderWare/RW34Active/rwsdk/d3d8/d3d8device.c#4 $
@@(#)$Id: //RenderWare/RW34Active/rwsdk/src/badevice.c#2 $
```

RenderWare 3.4 driven through **Direct3D 8**, fixed-function. This is what we replace: the same
assets, a D3D12/Vulkan-class renderer (D5).

### 4.1 Container census

| Container | Count | Size | Notes |
|---|---|---|---|
| `models/gta3.img` + `gta3.dir` | 6,043 entries | 327.5 MB | IMG **v1**: headerless `.dir`, 32-byte entries `(u32 sectorOffset, u32 sectorSize, char[24] name)`, 2,048-byte sectors, count = `dirSize/32` ✓ |
| — DFF | 4,617 | 136.5 MB | RenderWare Clump |
| — TXD | 1,368 | **182.6 MB** | the dominant asset class |
| — COL | 30 | 6.7 MB | collision |
| — IFP | 28 | 1.8 MB | animation |
| loose DFF / TXD / COL | 4 / 32 / 34 | — | loading screens, `models/*.TXD`, `data/maps/*/*.col` |
| `anim/ped.ifp` | 1 | 2.2 MB | magic `ANPK` |
| `anim/cuts.img` | 28 | 115 MB | cutscene animation (fidelity reference; cutscenes are out of scope) |
| `Audio/` | 1,221 | **1,021 MB** | `sfx.RAW` 340 MB + `sfx.SDT` 199 KB + 1,120 WAV + 90 MP3 + 9 `.adf` |
| `TEXT/*.gxt` | 5 | — | text for HUD/menus (missions out, text still needed) |

Trap: **6,043 entries but only 6,032 unique names** (11 names duplicated — `chef.dff`, `pgb.dff`,
`camera.txd`, `generic.txd`, …). Key by offset/order, never by name.

### 4.2 World size

| Metric | Value |
|---|---|
| Models / geometries | **4,613 / 6,505** |
| **Triangles** | **2,207,246** |
| **Vertices** | **2,818,890** |
| Morph targets | 6,505 (every geometry declares one) |
| Skinned meshes | **0** |

Geometry counts are **self-verifying**: every clump declares its geometry count in its geometry
list struct, and the declared total equals the number found (6,505, with zero mismatches across all
4,613 models). These figures supersede the first draft's 6,506 / 2,207,065 / 2,819,169, which came
from an ad-hoc walk rather than the verifier.

2.2 M triangles is a light load for a modern GPU — the modern-HW budget problem is **not** geometry,
it is draw-call count, overdraw, streaming and CPU simulation. Skinning being absent (ped/vehicle
bodies are rigid atomic hierarchies animated by per-frame matrices) removes another whole subsystem.

### 4.3 World data — what is populated, and what is empty

| Source | Content | Status |
|---|---|---|
| IPL `inst` | **9,114 instance lines** across 30 of the 32 `data/maps` IPL files | populated |
| IDE `objs` / `tobj` / `2dfx` | 3,789 object defs / 342 time-controlled objects / **1,069 2dfx entries** (31 area files + `maps/generic.ide`) | populated — 2dfx are lights/coronas/particles, i.e. **modernization material** (§5.7) |
| IPL `cull` blocks | — | **empty in all 30 area files**; the cull zones are in **`data/maps/cull.ipl` (631 zones)** |
| IPL `pick` | pickups | **empty in all 30 area files** — pickups are script-created ⇒ `vc_director` authors them |
| **`data/maps/paths.ipl`** | 1,957 blocks × 12 slots = 23,484, of which **14,732 points are in use** | the ground navigation graph, and the game loads it (`data/gta_vc.dat` line 83). Structure decoded, coordinate scale still a hypothesis — §4.11 |
| `data/paths/` | `flight.dat` (336 nodes), `flight2.dat` (158), `flight3.dat` (485), `spath0.dat` (79) | flight and script paths |
| `data/timecyc.dat` | **7 weather types × 24 hours = 168 rows**, 52 values in 23 columns (ambient, object-ambient, building-ambient, directional, sky top/bottom, sun core/corona, shadow, light-shadow, pole-shadow, **FarClp**, fog, `LightOnGround`, 3 cloud colours, blur RGB, water RGBA) | decoded (§4.13) — the master table for the modern atmosphere/sky model |
| COL | 64 files (30 IMG + 34 loose), **4,245 records parsed** of an estimated 4,272 | one file, `models/coll/peds.col`, is a known gap (§4.10) |
| `Cullzone.dat` / `Cullzoneempty.dat` | 50 zones (binary) | format probe at M0.5 |

`FarClp` in the sunny row reads **2000.0** — the original draws 2,000 units and fogs away the rest.
That single number is the most visible thing modernization buys back (§5.2).

### 4.4 DFF structure (measured)

```
Clump(0x10)
├── Struct(0x01)      numAtomics / numLights / numCameras
├── FrameList(0x0E)   hierarchy; 27,306 frame-tag chunks (0x0253F2FE) across the corpus
├── GeometryList(0x1A) → Geometry(0x0F) × N
│   ├── Struct(0x01)  u32 flags, u32 numTriangles, u32 numVertices, u32 numMorphTargets,
│   │                 then the attribute arrays — see §4.12
│   ├── MaterialList(0x08) → Material(0x07) × M
│   └── Extension(0x03) → 0x50E, 0x105   (GTA-specific extras)
└── Atomic(0x14) × N  frame index, geometry index, flags
```

Worked example: `landstal.dff`, 24 atomics/geometries; geometry 0 = `flags 0x10077, 106 triangles,
184 vertices, 1 morph target`. Counts are four adjacent `u32`s — directly readable. Flags select the
attribute mix (`0x2F` = positions+textured+prelit+normals+light; `0x77` adds tristrip/modulate).

### 4.5 No shader pipelines anywhere — materials are state, and we treat them as hints

Scanning every chunk in all 4,613 DFFs for Rockstar vendor IDs (`≥ 0x0253F2F0`) returns **only
`0x0253F2FE` (frame tags)**. There is **no Pipeline Set (`0x0253F2F3`)** and no shader blob. Materials
are fixed-function state: texture, filter/addressing, blend/dest-alpha, alpha test, cull, shade mode.

Consequence for a modernized engine: those state words are **not** something to emulate bit-for-bit;
they are *authoring hints* we translate into modern pipeline state and material parameters — while
keeping the ones that carry the art direction (alpha-test silhouettes on foliage/fences, dual-texture
detail overlays, prelit vertex colours).

### 4.6 TXD structure and texture formats

```
TextureDictionary(0x16)
├── Struct(0x01)   u32 textureCount   (== following 0x15 sibling count; verified 120/120)
├── TextureNative(0x15) → Struct(0x01)
│     u32 platformId = 8 (D3D8) · u32 filterAndAddressing · char name[32] · char maskName[32]
│     u32 rasterFormat · u16 width · u16 height · u32 level0ByteSize · …level data
└── Extension(0x03)
```

**All 11,997 textures are decoded and byte-exact** (probe 10, §15.2). The descriptor is 92 bytes;
level zero follows it directly, and **every later level is preceded by its own `u32` byte length** —
four bytes per extra level beyond the data itself. With that model every descriptor in the install
accounts for its payload exactly, and the census gates on it.

| Storage family | Textures | Share | Level zero |
|---|---|---|---|
| **BC1** (DXT1) | **10,824** | 90.2% | `w × h / 2` |
| **BC2/BC3** (DXT3 or DXT5) | **1,149** | 9.6% | `w × h`; the two share a byte budget, so sizes cannot separate them |
| **Paletted** | **24** | 0.2% | `w × h` plus a 1,024-byte RGBA palette |
| Unaccounted | 0 | — | — |

Every dimension is a power of two, and **895 textures carry a mip chain** — so the original does ship
mips, for 7.5% of the set rather than none. Two consequences for the cook:

1. BC1 and BC2/BC3 upload as-is, with no decode and no recompression, for 99.8% of the bytes.
2. Generating chains for the other 11,102 plus anisotropic filtering is still the cheapest visible
   image-quality upgrade available (§5.3), and the 24 paletted rasters are the only ones needing
   expansion before upload, because wgpu has no indexed colour format.

### 4.7 Audio

- `sfx.RAW` (340 MB) + `sfx.SDT` (199 KB table) — the SFX bank. **The samples are 16-bit PCM**,
  settled by measurement rather than assumption: adjacent-sample correlation over the first 4 MB is
  **0.80** as 16-bit words against **0.04** as 8-bit bytes, which only monotone 16-bit audio produces.
  The `sfx.SDT` table layout is still open — it does not read as a plain array of `(offset, length)`
  pairs, because a fifth of the slots carry `0xFFFFFFFF` sentinels and the offsets are not monotone.
- 1,120 loose WAV (PCM) + 90 MP3 (ambience/cutscene-adjacent music) — decodable today.
- **9 `.adf` radio stations**, 41–74 MB each (~490 MB). Probe 7 characterised them: every file starts
  with a 4-byte magic, and there are **two variants — `ddd9b24e` for the seven music stations and
  `ddd1aa4e` for the two talk stations** (K-Chat and VCPR). Entropy is 7.96–7.97 bits per byte, so the
  payload is compressed or lightly obfuscated, and there are a handful of **`ID3` tags and ~500 MP3
  frame syncs per MB** inside — enough to say there is MPEG audio in there, far too few to read it as
  plain concatenated MP3. This is a scoped reverse-engineering job on the container, not a dead end,
  but it stays the one item that may end as a documented gap (R6).
- `mss/mss32.dll`, `.m3d`, `Reverb3.flt` are Miles Sound System artefacts; we do not emulate them.

### 4.8 Correction from the first draft

`data/main.sc` and `data/debug.sc` are **leftover developer test scripts** ("Test script for
Industrial level"), not the game's mission sources. The first draft mislabelled them as shipped
Mission-Builder sources. With missions out of scope this only matters as a caution: there is **no
readable source** for the shipped mission logic, which is consistent with leaving it alone.

### 4.9 RenderWare stamps

DFF `0x0C02FFFF`; TXD `0x1003FFFF` — mixed within one install, so parsers must branch on the stamp
rather than pinning a version.

### 4.10 Corrections from the M0 verifier

The census tool replaced ad-hoc measurement with a checked one, and it overturned four conclusions
from the first pass. Recorded because two of them changed the plan:

| First-pass claim | Verified reality | Cause |
|---|---|---|
| "The ground nav graph is not shipped; the `path` blocks are empty" | **`data/maps/paths.ipl` is the shipped nav graph** (§4.11), and `data/maps/cull.ipl` holds 631 cull zones | The first pass globbed `data/maps/*/*.ipl` — depth 2 only — missed the two depth-1 files, and generalised from the empty per-area blocks |
| 6,506 geometries / 2,207,065 triangles / 2,819,169 vertices | **6,505 / 2,207,246 / 2,818,890**, declared == found, zero mismatches | The ad-hoc walk counted geometry sections anywhere in the tree rather than only under a geometry list |
| 3,511 objects / 314 time objects / 985 2dfx / 4,272 COL records | **3,789 / 342 / 1,069 / 4,245 parsed** | Same depth-1 miss: `maps/generic.ide`, and the COL files under `models/coll` |
| COL record layout understood | True for 33 of the 34 files | `models/coll/peds.col` does not fit the map layout — see below |

**Lesson kept:** any walk of the install must be recursive and must report what it skipped. The
census's `files_with_extension` does that, and its counts are what §4 quotes.

**Known gap, tracked as a probe:** `models/coll/peds.col` (21 KB, ~30 records) does not parse with
the map collision layout — its body is not `bounds + spheres + boxes + vertices + faces`. It is ped
collision data, needed at M3, not at M0. The census reports it as unparsed and counts its records by
magic marker so the total stays honest.

### 4.11 The path graph: structure decoded, scale still a hypothesis

Probe 2 (§15.2) took the graph apart. `data/maps/paths.ipl` is the only source — `data/gta_vc.dat`
line 83 loads it, and the per-area `path` blocks really are empty.

| Property | Verified |
|---|---|
| Block layout | **1,957 blocks**, each introduced by an unindented `<kind>, -1` header and holding **exactly 12** tab-indented points — 23,484 slots |
| Block kinds | kind 0 ×1,140, kind 1 ×709, kind 2 ×108 |
| Padding marker | **kind 0 means padding** — 8,752 slots, every one carrying a zero position. On a point in use, `next = -1` means *end of chain*, not an empty slot; conflating the two produced 1,915 false "dead links" before the probe caught it |
| Occupancy | **14,732 points in use**, 8,752 padding slots |
| Links | field 1 is the next point index within the block (`0..11`). 12,810 points link onward, 1,922 end a chain, and **every link resolves to a real point** (0 blocks with a dead link) — the model's own consistency check |
| Positions | raw fixed point, not metres. At **1/16** the points in use span x[-2,080, 1,501], y[-1,844, 1,828], z[0, 266] — inside the world box of the 9,114 placed instances (x[-2,153, 1,283], y[-1,967, 1,714]), with a street-level median of ~10 m |

The 1/16 scale is a **working hypothesis, not a decoded fact**, with three independent supports:
containment in the instance box, a plausible elevation, and the arithmetic that a signed 16-bit value
with four fractional bits spans exactly ±2,048 m — the extent of Vice City. `vc_paths::UNITS_PER_METRE`
carries it, and every point keeps its raw value beside the derived position, so a correction stays cheap.

Still open: fields 2, 6 and 7–11, and **how blocks interconnect** — links are intra-block only, so the
cross-block structure must live in one of the undecoded fields. That is now M5's main unknown.

### 4.14 Cull zones: 631 boxes, decoded

`data/maps/cull.ipl` holds 631 records of eleven values: an anchor, two corners, then two values whose
meaning is still open (40 and 0 in every record). The corners are the box; the anchor is not a third
corner:

| Property | Verified |
|---|---|
| Anchor x and y | **exactly the box centre on 630 of 631 records**, to within a hundredth of a unit. One record is 1.55 units off on x — a shipped-data anomaly, recorded rather than smoothed over |
| Anchor z | **not the box centre**: it ranges from the box floor to above 62% of its height, and for a tenth of the records it *is* the floor, so it is an independent height |
| Box size | median roughly 11 × 11 × 7 units — these are doorways, interiors and small occluded pockets rather than district-sized regions |

Consequence for M2: culling uses the corners, which are unambiguous, and treats `Cullzone.dat` (the
155 KB binary beside it) as a separate open question — the two may not carry the same zones.

### 4.17 Why probe 4 is answered without running the game

Probe 4 was going to measure how badly Vice City's simulation drifts with frame rate. **It cannot be
measured on the reference machine, and it does not need to be.**

The original **does not start on this machine at all**: it opens to a black screen with the audio
running normally. That is the classic failure of a 2002 Direct3D 8 title on a modern GPU, driver and
refresh rate — and it is a first-hand instance of exactly what this project exists to replace. Worth
recording plainly, because it is the clearest statement of the problem in the whole document: the
software this recreates cannot run on the hardware it is being recreated for.

The measurement is also unnecessary, for a reason that follows from D6:

- **The coupling itself is established.** The game ships a user-facing option called *Frame Limiter*
  whose purpose is to pin the simulation near 30 fps, and the one serious reimplementation of this
  engine still carries "Fix physics for high FPS" on its public to-do list — an independent project
  that ported the same logic and did not solve it either.
- **Quantifying the drift would be reproducing it.** We are replacing the timestep with a fixed 60 Hz
  core (§5.1), so the original's per-frame drift is not a behaviour to match — it is the defect being
  engineered away. Measuring it in detail would only be useful to someone choosing to preserve it.
- **The gate is ours to own.** D6 (§14.4) replays one input trace at 30/60/144/240 render rates and
  requires the simulation trajectory to match. That tests *our* build, on *our* hardware, and it fails
  closed. It is a stronger check than any observation of the original could provide.

So probe 4 closes as **confirmed by public evidence and superseded by D6**, not as skipped.

One knock-on: §14.5 expected reference captures from the running original at fixed camera poses. That
route is now blocked on this machine. The alternatives, in order: run `reVC` as the permitted
black-box host (D7) since it does start on modern hardware; use the original wherever it does run; or
fall back to asset-identity checks computed from the decoded data, and say so in the parity matrix.

### 4.22 The world is Z-up, and that is a rendering decision not a detail

Vice City's coordinates are **Z-up** with X east and Y north. Bevy is **Y-up** with -Z forward. Feeding
the data through unchanged would have rendered the entire city lying on its side, and it also put the
height axis in the wrong place in the first physics queries written for the walking player.

The conversion is a rotation, `(x, y, z) → (x, z, -y)`, applied on spawn: positions, rotations (the
axis moves, the angle does not) and per-axis scale. It is a rotation rather than a swap precisely so the
city is not mirrored — a reflection would look almost right and be subtly, permanently wrong.

Physics stays in the game's own axes, because collision data is authored there; only the render
boundary converts.

### 4.24 A review pass: six bugs, and the facts behind the foliage

Two reviewers were set on the runtime: one auditing the code, one establishing what the install says
about water and level-of-detail. Both found things worth recording.

**Bugs that made the picture wrong:**

| Bug | Effect |
|---|---|
| Streaming indexed a game-space grid with a Bevy-space camera position | Y (altitude) was read as north. The island is 18 cells deep and the runtime only ever streamed a column around z = 0 |
| Budget-limited cells restarted from their first instance | Hundreds of models spawned repeatedly at identical transforms |
| `AlphaMode::Mask` was inert for a custom material | Bevy discards alpha only in its own fragment path, so no cut-out ever happened: every palm and tree was a solid card |
| LOD shells drawn beside the detail they duplicate | **1,115 placements (12.2%)** overlapping real geometry |
| Texture cache keyed by address without holding the `Arc` | A recycled address could return a different image |
| Physics `place` bucketed by translation only | Rotated and scaled instances were registered where they are not |

**Water, from the data.** `data/water.dat` holds exactly one rectangle: `x, y ∈ [−3000, 3000]` at
**z = 6.0**. `WATERPRO.DAT` duplicates that and adds nothing recoverable. The texture is
`waterclear256` from `particle.txd`, and the tint is `timecyc.dat`'s last column — `WaterRGBA`, 46
distinct values across weather and hour. Pool and river water are separate map geometry, not this plane.
The ocean now renders as one quad at that height with that texture and tint.

**Level-of-detail, from the data.** There is **no flag** for LODs: the IDE flags column cannot separate
them (bit `0x100` appears on 43 ordinary models), the IPL has no LOD field, and all 9,114 placements
carry the same thirteen values. The only reliable rule is the name — 1,082 of 4,131 models contain
"lod", and no non-LOD model does.

**Vegetation, from the data.** Palms and trees are **meshes, not billboards**: `veg_palmbig14` is 88
triangles over 49 vertices spanning 16.6 × 17.0 × 18.3 units, built from mirrored pairs of flat cards.
They are collidable-free and their textures live in `generic.txd` and `gtatrees.txd` with alpha masks,
which is why the cut-out fix matters more than any other single change to how the city looks.

### 4.25 The resident renderer: rotation sense settled, whole map, physical light

The first playable slice streamed a 5×5 cell window and drew every material group as its own
entity. That is gone. The map is now baked once, whole, into merged chunks and lit by a real sun.
The measurements below come from `cargo run --release -p vc_game --example bake` on the reference
machine.

**The rotation sense is settled, and it was inverted.** The IPL quaternion is the *inverse* of the
rotation the game applied. The earlier `cargo xtask facing` tests could not see it because they
measured distances that a yaw mirror leaves unchanged. Measuring the lamp arm itself — the mean XY
of the top 20 % of each lamp model's vertices minus the bottom 20 % — against the vector from the
pavement path to the vehicle path separates the readings cleanly:

| reading | mean cos(arm, road) | arms over the carriageway |
|---|---|---|
| as stored | 0.132 | 60 % (201 of 605 lamps point the wrong way) |
| conjugated | **0.923** | **97 %** |

The stored reading is bimodal at ±1, which is what a mirror looks like: the arms stay collinear
with the road normal and half of them hang over the pavement. The conjugate collapses that to
576 of 605 in the top bin, and controls invert as expected (`bussign1` faces the pavement, 0 % vs
34 %). The fix is applied once, when the plan is built, so rendering, collision and any future
`2dfx` light placement all agree. The `R` toggle and the calibration lamps are deleted.

**Black patches were opaque alpha masks.** `vc_cook` classified a texture as a cut-out only when
some texels were clear and most were not. Vice City's baked shadow decals are the opposite: 95.9–100 %
of their texels are clear, their RGB is uniformly zero, and the alpha *is* the shadow. Classified
opaque, each one drew as a solid black card — 142 models, 238 placements, 2.8 % of the ground on
the whole map and about 7 % around the start cell. Textures now carry one of three behaviours:
`Opaque`, `Mask` (a real silhouette) or `Blend` (alpha as a shape). Blend materials also stop
casting shadows, which a solid quad otherwise does.

**The whole map is resident.** 7,421 placements bake into **71 chunks of 400 units**, merged into
one mesh per texture per chunk: **7,622 draws, 1,881,246 triangles, 2,377,086 vertices, 3,135
textures, zero cook failures**, in about 2 s on 12 cores. Merging is what makes this affordable:
the same geometry as one entity per material group is 27,404 entities, and per-model instancing
costs 21,950 draws across the map because almost every placement is a different model. A 2×2-cell
chunk also culls a street corner without splitting a texture into dozens of meshes; a 3×3 chunk
saves only 8 % of draws and culls much more coarsely.

Two data defects surfaced while filtering and are now handled by rule, not by hand:

| Defect | Scale | Rule |
|---|---|---|
| `interior` is not a reliable indoors flag: road, land and beach models are tagged `13` | 42 outdoor placements (166 k u² of ground) | keep any `interior > 0` placement whose position is shared by a LOD shell — shells exist only for outside scenery |
| Some LOD shells have no detail twin, so dropping all LODs leaves holes | 36 placements | keep those, except the four `IslandLOD*` shells, which are coarse stand-ins for detail drawn elsewhere |

**Lighting is physical now.** The single white fill is replaced by:

- a `DirectionalLight` at the hour's sun direction (east at 6h, highest at noon, west at 18h) with
  the timecyc's directional colour and 10,000 lux scaled by its brightness, four shadow cascades to
  600 units;
- a `GlobalAmbientLight` from the timecyc's ambient column, so shade is warm at noon and cool at
  night;
- HDR, ACES-fitted tonemapping (no LUT dependency), 4× MSAA, sRGB textures and prelit vertex colours linearised
  with a 0.06 floor. That floor is what stands in for the dynamic lights the original baked out:
  24 % of the world's vertices were black because a streetlight or headlight was doing the work.

The camera far plane is 12,000 units and the timecyc fog is stretched by 2.5 (to 5,000 at noon) so
the far shore is visible rather than pasted into a haze wall.

**Textures got their mips.** 5,465 of 6,075 unique textures ship with a single level; at a
5,000-unit view that aliases into noise. Mips are now generated in linear light, re-encoded as BC1
or BC2 (a bounding-box quantiser, mean channel error 1.1/255 on a decode-encode round trip) and
uploaded as sRGB with 8× anisotropy. The whole corpus is 105 MB compressed with mips; the map only
references 3,135 of them.

**Geometry with no normals.** 461 vertices across the map ship a zero-length normal, and a zero
normal shades black under a real sun whatever the albedo — a second, smaller source of the black
patches above. The batcher now fills any missing normal with the area-weighted normal of the
triangles using it, so `examples/bake` reports **zero normals: 0** instead of 461.

**Known gap.** 7,622 merged draws across the whole map is above §6's 1,500-draw worst-case target.
Frustum culling normally leaves three to four thousand visible, which a 5060 carries at 1080p, but
the target needs texture atlasing or meshlet-style indirect draws. That is the next rendering task,
not a regression hidden by a smaller view.

**Verification without a GPU.** `examples/bake.rs` runs the whole cook, mip and merge path headless
and prints the counts above, which is how the LOD, interior and normal fixes were checked.
`examples/shot.rs` renders a frame to an image through Vulkan's lavapipe, which is slow but needs no
display server.

### 4.28 The road lights are in the data, and we were dropping them

Every IDE file carries a `2dfx` section: the authored light for each lamp, sign and shop
window. The layout is `id, x, y, z, type, r, g, b, a, texture, shadow-texture, size, …`,
so the type, colour and size are all there. The parser kept only the offset, which is why
the city was lit by a global ambient term and nothing else.

| Type | Rows | What it is |
|---|---|---|
| 255 | 491 | street lighting, warm white `(255,255,200)` on 244 rows, blue signage on 49 |
| 184 | 68 | warm `(255,0,120)` |
| 185 | 53 | warm grey |
| 0 | 43 | assorted |

Instancing them across every placement gives **3,511 point lights** — far more than a
forward renderer should carry — so the runtime keeps the nearest **96**, rebuilt after the
camera travels 60 units, and switches them on outside 06:00-19:00 as the original did.
They cast no shadows: a lamp's own pool is what the eye expects, and shadow maps per lamp
would cost more than the whole world.

This is what the prelit shade floor (§4.26) was standing in for. The floor stays for the
daylight case, where the original's baked sun is the only light and a quarter of the
vertices are still near black because a dynamic light would have been added at runtime.

### 4.29 "The textures are sideways": five stages verified, one not

Reported repeatedly and chased wrongly four times. The corrections are worth recording as
much as the finding: the instance rotation sense was measured (lamp arms, cos 0.92 against
0.13), the road graph was measured, the LOD shells were cross-referenced, and the camera
basis was printed. Every one of those was either inconclusive or irrelevant to what the eye
saw, and each cost a rebuild.

What the data does say, each now covered by a check rather than an argument:

| Stage | Check | Verdict |
|---|---|---|
| TXD decode | `cargo xtask texture pizzaboy8bit128 out.png` | coherent character atlas |
| DFF texture coordinates | `cargo xtask uvs <model>` - `v` must track height | v tracks height on `jewler_shop`, `nbt_hotel06balcony`, `nbecland09` |
| Bake | test: a quad with u↔x, v↔z must survive `Batcher` | passes |
| Camera | startup log prints the basis | right (0,0,1), up (0.417,0.909,0), back (-0.909,0.417,0): zero roll |
| Mip chain | test: a black quadrant must stay in its corner, RGBA8 and BC1 | passes |

The remaining stage is the frame, which no statistic over the files can inspect. The
offscreen capture exists for that: `cargo run -p vc_game --example shot`. It had three
faults of its own before it worked - a duplicate `Msaa` panic, an HDR camera rendering into
an LDR target (a faithful black PNG), and silence about both - and now reads its own output
back and reports the mean brightness.

### 4.31 The bug behind "everything is sideways": vertices were never converted

Found in the end by the player's own screenshot, after six wrong answers of mine.

`Placement` converted an instance's position and rotation from the game's Z-up frame into
Bevy's Y-up, and `Batcher` then transformed the model's vertices — which were still in
Z-up. Every model was therefore placed correctly and *drawn in the wrong basis*: flat
ground (roads, pavements, terrain) came out standing on edge, buildings lay down. A vertex
has to take the same axis change as its position:

    world = A·(R·v + t) = (A·R·A⁻¹)·(A·v) + A·t

Verified by rendering two kilometres east of the island: a flat plane to the horizon with
Downtown upright, where before there were vertical sheets of pavement.

**Why every check missed it.** The texture decode, the DFF UVs, the model extents, the
instance rotations, the camera basis and the mip chain were each verified correct, and they
were correct — in model space or in placement space. The fault was in the *composition*
between those two spaces, which none of those checks looked at. A screenshot of the running
game was worth more than all of them, and is now the first thing to ask for.

### 4.30 What actually made the city unreadable: two lighting faults

"Everything is sideways" was reported for weeks and chased as a rotation five times. It was
never a rotation. What it was, found by rendering and bisecting with switches:

| Test | Frame mean | Conclusion |
|---|---|---|
| base | 40.1 | |
| every texture replaced by flat grey | 40.2 | not a dark image |
| shadow maps disabled | 40.5 | not the shadows at a distance |
| shadow-quad models dropped | 40.2 | not Vice City's baked shadow polygons |
| prelit floor 0.06 -> 0.38 | 41.8 | not the baked shade's floor |
| **ambient fill 600 -> 4000** | **52.1, min pixel 0 -> 11** | **found it** |
| world made `NotShadowCaster` | beach 9.4 -> **29.3** | **and a second one** |

Two faults, not one. Surfaces facing away from the sun are lit by the ambient term alone,
and 600 was far too weak; and the whole prelit world was receiving real-time shadows from a
sun it had already been shaded against, which cost a second pass over every visible chunk
as well as darkening it.

**Methods worth keeping.** WSL has lavapipe, so `cargo run --example shot` renders a frame
here and it can be looked at directly - the whole investigation took minutes once that
worked, after days of guessing. `VC_AMBIENT`, `VC_NO_SHADOWS`, `VC_NO_TEXTURES`,
`VC_NO_SHADOW_MODELS`, `VC_KEEP_ALL`, `VC_SHOT_EYE` and `VC_SHOT_TARGET` make each
hypothesis a render rather than a rebuild.

**A trap in the instrument itself.** The capture did not spawn the ocean, so sea rendered as
clear colour, and at a shoreline cell that is indistinguishable from missing ground. Three
readings of "the terrain is missing" came from that. The capture draws water now.

### 4.26 Modern lighting and draw budget

The resident renderer lights the map physically: a directional sun from `timecyc` with two shadow
cascades over 250 units, an ambient term from the same row, HDR with ACES tonemapping, and screen
space ambient occlusion. Cascades were cut from four to two and from 600 to 250 units because the
world is prelit — shadows read as contact detail near the player, and every extra cascade redraws
every visible chunk.

Fill is halved by culling back faces on everything except blended surfaces, which are the ones
genuinely drawn from both sides. MSAA is off to pay for the occlusion pass.

**Draw budget.** 7,622 merged draws for the whole map, three to four thousand after frustum
culling. That is above §6's 1,500-draw target; the remaining work is texture atlasing or indirect
meshlet draws, not more entities.

### 4.27 Cross-referencing rotations against the game's own LOD shells: negative result

The island LOD meshes are the original's baked copy of the same blocks that are placed in detail, so
they looked like the ground truth for the rotation convention: a detail building under the correct
reading should sit on the LOD surface.

Measured against `IslandLODmainland`, `IslandLODbeach`, `IslandLOD1` and `IslandLOD2`, sampling 300
detail instances each: **both readings land ~54 units from the nearest LOD vertex, and 1% within
2.5 units.** The shells are decimated to a few thousand vertices across a whole island, so vertex
spacing dwarfs any rotation error; the test cannot see the difference and was removed rather than
kept as dead tooling.

The convention is therefore settled the way §4.25 records: by the lamp arms themselves, which need no
external reference because the data carries both the arm and the road.

### 4.23 Collision: parsed, loaded, and not yet wired in

`vc_physics` holds static collision as *references*: a cell stores which collider sits where, and a
query transforms the model's triangles as it tests them, so the city's collision costs megabytes rather
than hundreds. The MVP queries are a downward ray for the ground and a directional cast for walls.

Loading found three things worth recording:

| Finding | Detail |
|---|---|
| Collision models | **3,825** across 30 `.col` files |
| `airport.col` has trailing junk | ~3 KB after the last record, so parsing is **lenient**: it stops and reports the leftover rather than losing the file |
| Many models have no collision **by design** | `veg_palmbig14` and `landstal` match no record at all — vegetation, vehicles and props are not collision-modelled this way, which is why 795 of 1,162 instances in the start region legitimately have none |

Coverage measured by probing straight down over every instance that *does* have collision: **223 of 367
hit**, with surface heights from -94 to 39.6 in game units — plausible for sea floor through bridge
deck, and the check that caught the height-axis bug above.

**The misses are explained, and they are legitimate.** Breaking down the 201 instances of the start
cell: **135 have no collision record at all** (vegetation, props — `veg_palmbig14` is not collidable in
Vice City), **42 point at 84-byte stub records** whose lists are all zero (signs and screens ship empty
COL entries), **23 have real faces** and one has a box. Nothing is misplaced; that cell is a beach.

The map as a whole is collidable: **179 of 226 cells contain instances with collision faces**, and the
richest are `(1, 1)` with 3,856 faces over 121 instances and `(-6, -1)` with 3,843 over 130. So a
walking test should spawn in one of those rather than on the beach, and the controller can be built on
the existing queries without a new data source.

### 4.21 The first playable slice: streaming around a free camera

A binary exists that streams the city. `cargo run -p vc_game` opens the install, indexes every texture,
then streams cells around a free-flying camera, cooking models as they come into range.

| Piece | What it does |
|---|---|
| `vc_stream` | Cooks models on demand; caches cooked models, dictionaries and textures; groups triangles by material slot |
| `vc_world::CellGrid` | Buckets the 9,114 placements into **230 cells of 200 units** and answers "what is near" |
| `vc_render` | One `Mesh` per material group, one material per texture, one GPU upload per dictionary texture |
| `vc_game` | Streams 5×5 cells, spawns entities with the instance's quaternion and scale, flies the camera |

Interiors are filtered out for now — they are a separate streaming problem, and the densest cell on the
map is a hotel interior, which makes a poor first view. That leaves **8,446 exterior placements in 226
cells**, and a radius-2 cook around the busiest of them is **878 models, 6,932 draw groups, 6,808
textured (98.2%)**, zero failures. The remaining 1.8% either name no texture (LODs and flat-colour
surfaces) or name a texture that no shipped dictionary holds.

Two bugs this step turned up, both found by measuring rather than by looking at a screen:

- **Chunk type `0x06` was missing from `rw_stream`'s container list.** Every texture-name walk returned
  nothing, so all **29,239** material slots were nameless while the geometry, the slot count and the
  textures themselves all looked fine. One line.
- **Vice City duplicates shared textures across many dictionaries** and points a model at only one of
  them (`white64` lives in dozens of TXDs). A global index built once at startup — 1,362 dictionaries,
  12,010 textures — takes resolution from 89% to 98.2%, at the cost of a warm-up pause before the
  window opens. Doing it lazily instead put a multi-second stall in the middle of streaming.

### 4.20 IDE and IPL layouts, and map coverage

The world layer parses both text formats into types and resolves them against the archive. Three field
layouts the first draft would have got wrong:

| File | Fields | Layout |
|---|---|---|
| IDE `objs` | **6** | id, model, TXD, object count, draw distance, flags |
| IDE `tobj` | **8** | the same, then on-time and off-time before the flags |
| IPL `inst` | **13** | id, model, **interior**, position, scale, **quaternion** |

Two details worth recording. `interior` is **-1 for "no interior"** — exactly one shipped placement
uses it, and declaring the field unsigned dropped the whole 61-placement file it lives in. And the
rotation is a **quaternion** (`x y z w`), not the Euler triple the earlier games used.

Coverage, gated by the census: **4,131 definitions against 9,114 placements**, with **zero** placements
naming an unknown id, **zero** definitions whose DFF is missing from the archive, and **zero** whose
TXD is. Every model and every texture dictionary the map refers to exists and is named consistently,
which is the precondition M2 needs before streaming can be built on it.

### 4.15 Spawn policy: what the data gives us and what it does not

Probe 3 asked how much of the ambient spawning was script-driven. Split by actor:

| Actor | Source | Verdict |
|---|---|---|
| **Pedestrians** | **`data/pedgrp.dat` is data-driven**: its own header says it "contains the list of ped groups for each zone. Each ped group should contain 16 ped type names", and the file holds about 70 such groups | Spawning per zone needs **no script**: zone from `info.zon`, group from `pedgrp.dat`, stats from `pedstats.dat`, relationships from `ped.dat` |
| **Vehicles** | `data/default.ide` defines the **car classes** (`NORMAL_CAR 1`, `POOR_FAMILY_CAR 2`, `RICH_FAMILY_CAR 4`, …) but no file assigns a class mix or density to a zone | The **density and mix were scripted or engine-side**, so `vc_director` must author them — this is the part of R4 that survives |
| **Pickups** | IPL `pick` blocks are empty (already known) | Script-created; authored by the director |

So the answer to R4 is narrower than feared: peds are data-driven end to end, and only vehicle density needs authoring.

### 4.16 Cullzone.dat: the binary mirror of cull.ipl

`data/Cullzone.dat` is the same 631 zones in binary. The first non-zero byte sits at offset 24,580 — a zeroed prefix, not a header — and the floats from there reproduce the first IPL record exactly (`-357.922`, `-335.663`, `1058.65`, `1106.74`, `6.47`), but ordered as **axis pairs** `(min_x, max_x) (min_y, max_y) (min_z, …)` instead of corner triples, preceded by a `u32` that is 76 on the first record.

That makes the file **redundant for us**: the decoded text form is easier to verify (§4.14), so M2 reads `cull.ipl` and this file stays a cross-check rather than a dependency.

### 4.18 The cook: both asset halves verified on the whole corpus

M1's asset side is done. `vc_cook` turns decoded data into artifacts a GPU takes as-is, and both
halves are verified against every shipped asset rather than a sample:

| Half | Verification |
|---|---|
| **Meshes** | All 6,505 geometries decode to **2,207,246 triangles and 2,818,890 vertices** — exactly the declared totals — with zero out-of-range indices and zero non-finite positions |
| **Textures** | All 11,997 cook with zero failures into **179,522,336 bytes**: 10,824 BC1, 1,149 BC3, 24 expanded to RGBA8. Mip chains are split on the per-level length prefix; block data is never decoded |

Two accounting notes worth keeping, because both look like bugs otherwise:

- Cooked bytes **exceed** the source by 539,712. Paletted rasters grow four-fold when expanded to
  `RGBA8`, which more than pays for the mip length prefixes the cooker strips. Both totals are gated
  so the difference stays explained.
- **BC2 versus BC3 is settled: they are BC3.** The two are indistinguishable by size, but 80.4% of the
  16-byte-block textures open with an equal alpha-endpoint pair — the DXT5 signature for mostly-opaque
  data, where DXT3's raw nibbles would manage it about 0.4% of the time.

What M1 still needs is the runtime: a Bevy app that streams these artifacts and draws them.

**The artifact format, and a decision change.** `cargo xtask cook landstal.dff --txd generic.txd`
writes a `.vcck` file and reads it back to verify the write: magic `VCCK`, a `u32` **format word**,
then a `postcard` payload. The first artifact — the flatbed — is 24 meshes, 3,803 vertices, 2,529
triangles and 13 textures in 533,616 bytes.

§6.2 originally called for hand-rolled `bytemuck` POD for cooked data and `postcard` only for saves.
That split is **dropped**: `postcard` over our own serde types gives the same on-disk stability the POD
decision was protecting (the schema is ours, the format word guards it), for a fraction of the code,
and it was verified by round-tripping a synthetic scene in tests and the real model on disk. A POD
layout remains available later if load time ever demands it, and the format word makes that swap
visible rather than silent.

### 4.19 The runtime slice: compile-verified, not yet run

`vc_render` and `vc_game` exist. The `vc-rs` binary reads a `.vcck`, uploads every cooked mesh as a
Bevy `Mesh`, uploads textures **in the format the cook produced** — `Bc1RgbaUnorm`, `Bc3RgbaUnorm`,
`Rgba8UnormSrgb`, so block data is never decoded on the CPU — gives them one custom `Material`, and
frames the result with a camera.

**What is proven and what is not.** The Rust compiles under the full gate. The WESL fragment shader
is *not* proven: shaders are only compiled when a pipeline is created, which needs a GPU, and this
machine has no GPU path (§16). The first `cargo run -p vc_game` on Windows is therefore the real test
of the shader, not a formality. That distinction is recorded here so a future session does not read
"gate passes" as "the renderer works".

```bash
cargo xtask cook landstal.dff --txd generic.txd   # WSL: writes vc-artifacts/cooked/landstal.vcck
cargo run -p vc_game                              # Windows: first pixels
```

Deliberately minimal for this step, each a named next increment rather than an omission: one material
for the whole scene (per-triangle material ids are cooked and carried but not yet honoured), no prelit
vertex colour in the shader, no lighting, no mip levels beyond level zero uploaded.

### 4.13 Timecyc: decoded, and the draw distance varies with weather

`vc_timecyc` reads all 168 rows with no failures, and the column order the file's own header names
accounts for exactly the 52 values each row carries — so the parse is self-checking rather than
guessed. Two things fall out of it that matter to §5.2:

- **The far clip plane is not a constant.** It runs from **150** units in the heaviest weather to
  **2,000** in clear sun. The original does not merely fog away a fixed distance; the fog *is* the
  weather, so extending the view has to preserve that curve rather than flatten it.
- Elevation, cloud, water and blur colours are all per-hour, which is what makes the modern
  atmosphere able to stay recognisably Vice City while the renderer underneath is new.

One parsing trap worth recording: every weather section repeats the column-name header as a `//`
comment, so a parser that treats any `//` line as a section marker sees 7 real weather types plus
~200 phantom ones. Only a run of four or more slashes introduces a section.

### 4.12 DFF geometry layout: decoded and byte-verified

Probe 6 (§15.2) was the M1 cook blocker: without the vertex encoding there is no mesh conversion at
all. It is now decoded, and the decoding is **verified by byte accounting on every shipped geometry** —
6,505 of 6,505 structs are fully accounted for, with zero failures. `vc_dff::read_layout` implements
it and the census gates on the result.

The struct, in file order (all offsets from the start of the geometry's struct payload):

| Field | Size | Notes |
|---|---|---|
| `flags`, `numTriangles`, `numVertices`, `numMorphTargets` | 4 × `u32` | the four words §4.4 already had |
| **surface properties** | 12 | ambient RGB as three `f32`s, always `1.0` in shipped data — **present only for stamps below `0x1003FFFF`** |
| prelit colours | `4 × vertices` | `RGBA` as four `u8`s, present when flag `0x08` |
| texcoords | `8 × vertices × sets` | two `f32`s per set; one set with flag `0x04`, two with `0x80` |
| triangles | `8 × numTriangles` | two `u32` words: three `u16` indices, the third in the high half of the second word, and the material id in its low half |
| per morph target | 16 + 8 | bounding sphere as four `f32`s, then `hasVertices` and `hasNormals` as two `i32`s |
| positions / normals | `12 × vertices` each | three `f32`s, present only when the corresponding flag word is non-zero |

Two corrections to earlier assumptions, both caught by measurement rather than reasoning:

1. **There is no `ambient/diffuse/specular` RGBA block** as the first draft implied. What looked like
   one was the prelit colour array in one case and the surface-properties record in another.
2. **The 12-byte surface-properties record is stamp-dependent.** Geometries stamped `0x0800FFFF` or
   `0x0C02FFFF` carry it; those stamped `0x1003FFFF` do not. Assuming it unconditionally left 1,919
   geometries misaligned by exactly 12 bytes, and ignoring it entirely left the other 4,586 short.

The layout was cross-checked against MIT `librw`'s reader, which is the oracle §10.1 sanctioned for
this question: `librw` reads the same fields in the same order, and the byte accounting agrees with it
on the whole corpus. Feature totals that fall out of it: **4,185 geometries carry prelit colours** and
**2,331 carry normals** — so roughly a third of the model set has no prelit data and needs the
timecyc-driven lighting path rather than baked vertex colour.

---

## 5. Modernization plan

The heart of this revision: what the original does, what we do instead, and what it buys.

| # | Original (2002) | Modern approach | Gain |
|---|---|---|---|
| 5.1 | **Logic coupled to frame rate**; VC needs its ~30 fps limiter, and high frame rates break timers/physics | **Fixed-step simulation** (60 Hz) fully decoupled from rendering; render interpolates transforms; every timer expressed in ticks; input sampled per frame, consumed per tick | Stable behaviour at any refresh rate — the prerequisite for "work well on modern hardware" (D6) |
| 5.2 | **`FarClp` = 2,000 units**, fog used to hide the cut | Extended far plane (**5,000–8,000**), LOD chain (existing `lod*.dff`), **distant-city impostors/HLOD baked in the cook**, modern atmospheric fog driven by timecyc's fog/sky columns | Vice City reads as a city instead of a fog bank; the biggest visual change |
| 5.3 | **Mip chains on only 7.5% of textures** (895 of 11,997); point/linear filtering; DXT1 and DXT3/5 | **Generate mip chains in the cook**, anisotropic filtering, trilinear; BC7 for alpha-heavy textures (2× VRAM, ~366 MB — affordable) | Kills 2002 shimmer on roads, fences, foliage, railings |
| 5.4 | **Immediate-mode per-object draws**, D3D8 | Bevy 0.20 already batches repeated mesh+material pairs and **runs GPU preprocessing + indirect draws by default**; the cook dedupes geometry so instances share mesh handles; our CPU work is limited to per-cell streaming/LOD decisions | Draw calls collapse to hundreds — and this is configuration, not engine work (§20) |
| 5.5 | Cull zones + occluder boxes, BSP-less CPU culling | **Bevy 0.20 ships two-phase GPU occlusion culling** (`OcclusionCulling` + depth prepass, hierarchical Z-buffer, no baking); enable it and measure; `Cullzone.dat` retained as a coarse streaming hint | Long draw distance without paying for it — again configuration rather than engine work (§20) |
| 5.6 | Fixed-function shading; no shadows beyond a shadow blob | Custom `Material`s on Bevy's pipeline: cascaded sun shadows, spot shadows for headlights, alpha-test/dual-texture preserved, `AlphaMode`/blend from RW state | Modern shading that keeps the art direction |
| 5.7 | Prelit vertex colours + a handful of dynamic lights | Prelit = baked indirect term (unchanged), **plus** dynamic sun/sky from timecyc and **clustered local lights driven by the 985 2dfx entries** (street lamps, neon, coronas become real lights) | Night-time Vice City modernized — the neon city finally lights itself |
| 5.8 | Flat water plane, cheap animation | Modern water: animated normals, reflection/refraction, depth tint, timecyc `WaterRGBA`; polygons still from `water.dat`/`WATERPRO.DAT` | Modern look, same geography |
| 5.9 | Synchronous IMG streaming, TXD-from-archive | **Async cooked-artifact streaming** (background threads, residency budgets, cell/frustum priority); cooked artifacts are compile-time-optimized (mips, BCn, quantized vertex streams, GPU-ready layouts) | No hitches, faster loads, controlled VRAM |
| 5.10 | All AI ticked uniformly | **AI LOD**: tick rates by distance/visibility; budgeted ECS job scheduler; cheap steering for distant peds | Hundreds of active peds without CPU spikes |
| 5.11 | 2002 node graph, **shipped as `data/maps/paths.ipl` (25,441 nodes)** | Read the original graph, then extend it: hierarchical A*, flow fields, path cache, and a generated navmesh over collision where the node graph has no coverage | Modern pathfinding that starts from the game's own connectivity instead of guessing at it |
| 5.12 | Miles S3D, frame-driven audio | Own mixer on `cpal`: 48 kHz float DSP, spatialization, occlusion/low-pass, reverb, streamed radio | Modern audio; only the `.adf` codec is unresolved |
| 5.13 | 4:3 HUD, keyboard-only, no rebinding | Widescreen-aware HUD/radar, gamepad + mouse look, rebinding, high-DPI text, in-game settings | Playable on a modern desktop |
| 5.14 | 32-bit, Windows-only, DirectX 8 | 64-bit, multithreaded, **wgpu** (D3D12 on Windows, Vulkan on Linux) | One shader path, modern APIs. D3D12-specific features (mesh shaders, bindless, work graphs) are a stretch behind wgpu feature flags, not a foundation |
| 5.15 | Modding = IMG editing | Loose-file override + cooked-format mods + HD texture pack support | Modern mod path; the community can modernize the art further |

---

## 6. Performance targets and budgets

Concrete, measurable, and checked at every milestone (§14.2):

**Reference machine (this box, §16):** RTX 5060, **8 GB VRAM**, 12 cores, RTX-class mid GPU
(driver 610.x). Targets are stated for that machine; anything above it is headroom, not a promise.

| Metric | Target |
|---|---|
| Frame rate | **144 fps at 1440p** in the densest area; 1080p as the high-headroom case | 
| 4K | Stretch only — 8 GB VRAM makes a 4K texture/mip budget the binding constraint, not GPU throughput |
| CPU frame | ≤ 6.9 ms (144 Hz) — sim tick, streaming, ECS (culling is largely on the GPU, §20) |
| Sim tick | 60 Hz fixed, ≤ 4 ms with ~120 active peds + 40 vehicles |
| Draw calls | < 1,500 in the worst-case view, achieved via mesh-handle batching + indirect draws |
| Streaming | no hitch > 4 ms; cold start to playable < 10 s |
| VRAM (textures) | ≤ ~1.5 GB at 1440p with BC7 + mips (base data is only 183 MB) — must leave room in 8 GB |
| Behaviour stability | identical sim trajectory at 30/60/144/240 render fps (D6 test) |

---

## 7. Architecture

Rust 2024, `resolver = "3"`, all tooling in `cargo xtask` (no shell scripts).

```
crates/
  vc_transport      install discovery + read-only IMG v1/DIR reader (opens files, knows no formats)
  rw_stream         RenderWare chunk-tree reader, shared by DFF/TXD/IFP (parses, opens nothing)
  vc_dff vc_txd vc_col vc_ifp vc_ipl vc_ide vc_gxt vc_dat
                    one crate per format family → typed structs, never raw bytes
  vc_ir             engine-neutral IR (meshes, materials, world, anim, collision)
  vc_cook           IR → cooked artifacts; mips, BCn, impostors, **navmesh**, content-addressed cache
  vc_asset          Bevy AssetLoaders for cooked artifacts; async streaming + residency budgets
  vc_render         custom Materials on bevy_pbr's mesh pipeline; culling pass; water; impostors
  vc_world          cells, streaming policy, LOD, cull zones, occlusion, time-of-day/weather
  vc_sim            fixed-step core: collision, player, vehicles, combat, damage
  vc_ai             ped/vehicle/police task & event model, nav queries, AI LOD scheduler
  vc_director       authored ambient policy replacing script-driven spawn/ambient behaviour
  vc_audio          SFX bank, MP3/WAV/ADF decode, mixer, spatialization
  vc_ui             HUD/radar, menus, settings, GXT-backed text
  vc_game           composition root; the only crate that names every other one
  xtask             cook / census / export-gltf / navbuild / loc / perf / parity-check / bump-format
```

Rules carried over because they earned their place:

- **Three floors, one door.** `vc_transport` opens files; format crates open nothing; `vc_cook` and
  `vc_asset` build products; `vc_game` exposes one `load_session()`.
- **Typed refusals.** Unsupported material state, missing animation, unreadable zone → a typed gap
  enum, never a silent default.
- **One simulation funnel.** `step(world, tick, input) -> TickResult`, so live play, headless tests
  and replay share a single path (also how D6 is verified).
- **Measurement is a deliverable.** `cargo xtask loc|perf`, frame-budget reports in-process, hitch
  counters; perf regressions are test failures, not anecdotes.

---

## 8. Subsystem designs

### 8.1 Renderer (modern, VC art)

- Custom `Material` impls on `bevy_pbr`'s mesh pipeline (WESL shaders, Bevy 0.20 — see R10).
- Vertex streams keep prelit colours (day + night), 2 UV sets and geometry flags; prelit is the
  baked indirect term, so the city never looks "re-lit from scratch".
- RW state → pipeline: alpha-test → `AlphaMode::Mask` (foliage/fences silhouettes preserved);
  additive/modulate/dest-alpha → explicit engine blend state; dual texture → second UV set + sampler.
- Atmosphere: timecyc drives sun/sky/fog/water numerically; 7 weather types × 24 hours interpolated.
- Shadows: cascaded sun shadows; spot shadows for headlights; 2dfx lights feed the cluster.
- Water, impostors, tonemapping (HDR), MSAA + optional TAA.
- Textures: BC1/BC3/BC7 with generated mips and anisotropic filtering, streamed with budgets.

### 8.2 World and streaming

Cells follow the game's own `data/maps/<area>/` grouping (so LOD and culling granularity match the
original). Per cell: instance transforms, distance LOD swap using the `lod*.dff` chain, frustum cull,
then occluder/hi-Z cull, then impostors past the LOD range. `Cullzone.dat` and IPL data inform
priority only. Streaming is async with a residency budget and eviction; the cook's compact artifacts
are what get streamed.

### 8.3 Simulation

- **Fixed step at 60 Hz**, decoupled from rendering; render interpolates transforms (D6).
- **Collision**: BVH built at cook time over COL triangles/spheres/boxes with `SurfaceProperties` →
  friction, audio, particles. Queries (line/sphere/capsule) run through **`parry3d`** (Apache-2.0,
  QBVH + ray/point/contact algorithms); parallel across the ECS. The *response* (arcade, GTA-style
  sliding/stepping) is ours, not the crate's.
- Player on foot: the original's movement model — snap/slide/step/stair handling — re-derived as a
  timestep-based controller.
- Vehicles: `handling.cfg` values used verbatim, fed to a modern-but-equivalent arcade model
  (4 wheel traces, suspension, grip, damage), stepped at a fixed rate. No FPS dependence.

### 8.4 AI (the port)

- Architecture: task/event model (task trees, event groups, relationship matrix from `ped.dat`),
  ported at behaviour level onto ECS components with a budgeted scheduler.
- Data used verbatim: `pedstats.dat` (attributes), `pedgrp.dat` (zone groups), `ped.dat`
  (relationships), `weapon.dat`, `carcols.dat`, IDE `cars`/`peds` definitions in `default.ide`
  (model-id ranges: peds 0–129, vehicles 130–239, wheels 250–257, weapons 259–294).
- Navigation: the shipped `data/maps/paths.ipl` node graph (25,441 nodes) as the base
  connectivity, with hierarchical A*, flow fields for crowds and a per-ped path cache; a cook-time
  navmesh over collision fills the gaps the node graph cannot express (R2, §4.10).
  for crowds, per-ped path cache; replanning under budget.
- AI LOD: full task ticking near the player, cost-reduced steering/velocity matching at distance,
  population parked beyond a radius (with correct re-entry).
- Traffic: lane/graph following, junction rules, avoidance, parking, spawn/despawn from density.
- Police: escalation ladder driven by the wanted level, pursuit and roadblock behaviour.
- Animation: ANPK/IFP keyframes over rigid atomic hierarchies — no skinning anywhere in VC.

### 8.5 Audio

Own mixer on `cpal` (48 kHz float), voices with distance attenuation, occlusion, low-pass and reverb;
engine/tyre/ambient loops fed from the sim; radio streamed from `.adf` if R6 resolves, otherwise a
documented gap with the 90 MP3s and the SFX bank still working. `vc_director` drives ambience.

### 8.6 UI, text, input (decided)

- **Fonts**: the original HUD text is **bitmap glyph textures inside `models/fonts.txd`**, not a TTF.
  Decision: decode those glyphs at cook time into an atlas plus per-glyph metrics, and render HUD text
  with our own quad/layout path so the HUD keeps its identity. `bevy_text`/`bevy_ui` are used for
  menus, settings and developer overlays where a modern scalable font is an improvement, not a loss.
- **Radar**: the minimap art ships as `radar*.txd` inside the IMG. Decision: assemble the radar tile
  set into a cook-time atlas (with the zoom levels the original used) and draw blips ourselves.
  `fronten1.txd`/`fronten2.txd`/`hud.txd`/`MISC.TXD` are decoded the same way as any other TXD.
- **Input**: `bevy_input` for keyboard/mouse plus `bevy_gilrs` for gamepads; bindings are data
  (a TOML table in settings), not hard-coded, so rebinding and a second input device cost nothing.
  Mouse look is native — no cursor-lock hacks from 2002.
- **GXT text**: our own parser (`vc_gxt`), 5 shipped languages, used for ped names, vehicle names,
  radio station names and menu strings. Missions are out of scope, so no mission strings are needed.

---

## 9. Risk register

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| R1 | **AI port scope** — ped task/event AI, traffic, police, combat is the largest remaining subsystem, now that the mission VM is gone | **High** | Behaviour-level port on ECS with a budget; land it in stages (M5 peds → M6 traffic/police → M7 combat), each with its own gate. Numbers come from the shipped data files, only the implementation is new. |
| R2 | **How path blocks interconnect is undecoded** — the graph is shipped and its block/point/link structure is decoded (§4.11), but links are intra-block only and the coordinate scale is a hypothesis (1/16) rather than a decoded fact | Medium | M5 must either find the cross-block fields or build connectivity from proximity; `vc_paths` keeps raw values beside derived ones so a scale correction is cheap. Downgraded from **High** on 2026-10-26, when the file was first located. |
| R3 | **Framerate-dependence retrofit** — the original's logic and timers are frame-coupled, and it ships a *Frame Limiter* option to keep the simulation near 30 fps | **High** | The coupling is confirmed rather than measured (§4.17): the original will not even start on the reference machine, so D6 — replaying one input trace at 30/60/144/240 render rates and requiring identical trajectories — is the gate, and it tests our build rather than the original's. |
| R4 | **Spawn/ambient policy was partly script-driven** (IPL `pick` and `cull` blocks are empty; pickups are script-created) | Medium | `vc_director` authors it; M0.5 estimates how much. Recorded in the fidelity ledger as authored, not ported. |
| R5 | **Modern lighting can make 2002 art look wrong** (over-lit, plastic) | Medium | Prelit colours remain the base term; lighting is additive; keep a "period" toggle for A/B comparison against the original. |
| R6 | **`.adf` radio** — two container magics, 7.97-bit entropy, embedded ID3, ~490 MB | Medium | Probe 7 (§4.7) narrowed it to a container problem with MPEG audio inside, which is progress but not a decoder. Failure ⇒ documented gap (iw4L precedent: report gaps, never synthesise). |
| R7 | **DFF vertex encoding** — ~~undecoded~~ **decoded and byte-verified 2026-10-27** (§4.12): all 6,505 geometry structs are fully accounted for and the census gates on it | ~~Medium~~ Closed | Kept as a note: the surface-properties record is stamp-dependent, the one detail a future reader is likely to re-derive the hard way |
| R8 | **Offline cook vs. semantic loss** | Medium | Cooked formats store material state words and vertex streams verbatim; glTF is a debug lens only. |
| R9 | **Install drift** — 1.0 versus 1.1, retail versus Steam, or a repack would change the assets and silently move every measurement | Low | The census fingerprints the archive directory and `--verify` fails on a mismatch, so it has to be updated deliberately. Legal exposure is handled in §12. |
| R18 | **Legal exposure** — Take-Two DMCA'd comparable projects (`re3`/`reVC`, blocked since 2021) | Medium | Source-only distribution, no assets ever, clean-room discipline: §12. Unchanged by anything in M0. |
| R19 | **24 paletted textures need expansion in the cook**, because wgpu has no indexed colour format (§4.6) | Low | Expand to `RGBA8` at cook time; it is 0.2% of the set so the memory cost is noise. |
| R10 | **Bevy 0.20 specifics**: WESL shader syntax replaced `#import`/`#ifdef`; instancing/batching assumptions and `bevy_pbr` custom-material hooks need confirming at M1 | Low–medium | Verify hooks at M1 with one material before scaling out; shaders authored in WESL from day one. |
| R11 | **Impostor/HLOD authoring is new content work** — no distant-city LODs ship beyond the `lod*.dff` chain | Low–medium | Cook-time generation from existing LOD geometry; measure against 5.2's target. |
| R12 | **No hardware GPU inside WSL** — the WSL driver exposes CUDA/D3D12 bits but no NVIDIA Vulkan ICD (only Mesa software `lvp` and other vendors' drivers) | **High** (blocks "run it and see") | Verified 2026-10-26. Rendering runs and is measured **on Windows native (D3D12)**; WSL stays the CPU-side cook/test/CI environment. `lvp` software Vulkan is available for headless image tests, but is not representative for perf. §16 |
| R13 | ~~**No Rust toolchain installed anywhere**~~ — resolved 2026-10-26: WSL has rustup with 1.99.1 pinned, Windows has rustup 1.29.1 | ~~High~~ Closed | Kept as a note: bump the pin deliberately when Bevy moves its MSRV |
| R14 | **Bevy instancing requires identical mesh handles** — different mesh assets never share a batch, so a naive one-mesh-per-instance cook destroys the draw-call win | Medium | Cook dedupes geometries and maps IPL instances onto shared meshes; assert batch counts in the M2 perf harness. §20 |
| R15 | **Two-glam-version trap** — Bevy 0.20 requires `glam ^0.33.2`; depending on latest (0.34) puts two glam versions in the graph and breaks type interop | Medium | Pin `glam = "0.33"` workspace-wide (verified requirement). §17 |
| R16 | **CI cannot contain game-derived fixtures** — golden vectors, captured reference frames and census inputs all derive from copyrighted data | Medium | Synthetic fixtures in-repo for CI; every game-data test is local-only behind `VC_GAME_ROOT` and writes to ignored `vc-artifacts/`. Enforced by review + `.gitignore`. §18 |
| R17 | **Encoder build toolchain** — `ctt`'s best backends compile C/C++ and need `cmake`/`clang`, which are **missing** on this box; and `cargo-deny` cannot detect a crate whose declared licence differs from what it links | Low–medium | Install `cmake` + `clang` in M0; keep the encoder in the cook path only (never in the runtime binary); manual review of anything linking system libraries. §16, §17 |

Removed since the first draft: the SCM v2 VM / mission-script risk (was the top risk) — now out of scope (D1).

---

## 10. Milestones

M0–M4 is the "modern free roam, playable" spine. M5–M7 makes the city *alive*. M8–M10 complete and
optimize. Every gate includes the §6 targets that apply to it.

| # | Milestone | Gate |
|---|---|---|
| **M0** | **Recon + verifier** ✅ 2026-10-26 | `vc_transport` + `rw_stream` + `cargo xtask census --verify` reproduces §4 exactly: 6,043 IMG entries with 11 duplicate names; 4,617/1,368/30/28 by extension; 4,613 models with 6,505 geometries (declared == found) and 2,207,246 triangles; 11,997 textures across 1,361 dictionaries; 9,114 instances, 631 cull zones, 25,441 path nodes; 3,789 objs / 342 tobj / 1,069 2dfx; 34 COL files with 4,245 records and exactly one known-unparsed file. Mismatch fails the build. |
| **M0.5** | **Probes** (time-boxed) | Resolve: nav-graph source (R2), spawn-policy scope (R4), framerate-coupling confirmation (R3), full TXD format histogram, DFF vertex encoding (R7), `sfx.SDT` codec, `.adf` verdict (R6), `Cullzone.dat` layout, timecyc column semantics. |
| **M1** | **First pixels, modern** | Cook one area (mips + BCn), custom materials on the Bevy pipeline, timecyc sky/fog, free camera. Gate: reference screenshot vs. the original for *asset identity* (texture, colour, silhouette) **plus** 5.3 mips visibly removing shimmer, at 144 fps 1440p. |
| **M2** | **Whole world** | IPL/IDE → cells, async streaming, BVH + occlusion culling, extended far plane (5.2), LOD + impostors, water, day/night + weather. Gate: traverse the entire map at 144 fps 1440p with no holes, no pop beyond the authored LOD range, correct 24-hour cycle. |
| **M3** | **Player on foot, fixed-step** | COL → BVH, arcade character controller, stairs/curbs/falling, fixed-step + interpolation. Gate: walk the whole map without falling through; D6 test passes (identical trajectory at 30/60/144/240 render fps). |
| **M4** | **Vehicles** | `handling.cfg` model, car COL, wheel/dummy hierarchy, damage, radio plumbing. Gate: drive everywhere; handling feels like the original; still framerate-independent. **"Modern free roam playable" achieved.** |
| **M5** | **Ped AI** | Nav build (R2), task/event model, stats/groups/relationships, animation. Gate: peds traverse and behave plausibly city-wide inside the sim budget (≤4 ms tick, 120 peds asp). |
| **M6** | **Traffic + police + wanted** | Vehicle AI, junction rules, spawn/despawn from density, escalation ladder, pursuit. Gate: self-sustaining traffic with no script; police chase behaves; AI LOD evidence (tick-cost histogram). |
| **M7** | **Combat + world interaction** | Weapons, damage, pickups (director), particles, object breaking, explosions, 2dfx effects. Gate: complete combat loop against peds/police/vehicles; performance holds under fire. |
| **M8** | **Audio** | SFX bank decode, mixer, spatialization, engine/tyre/ambient, radio if R6 resolved. Gate: audio-event coverage table with no silent failures. |
| **M9** | **UI, settings, save** | Widescreen HUD/radar, menus, settings, free-roam save/load, gamepad + rebinding. Gate: playable end-to-end with keyboard and gamepad, high-DPI, ultra-wides. |
| **M10** | **Optimization + polish** | Profiling passes against §6, GPU-driven/indirect stretch, optional upscaling, mod support, cross-platform (D3D12/Vulkan), packaging. Gate: all §6 targets met on the reference machine. |

---

## 11. Old vs. new plan

| First draft | This revision |
|---|---|
| Full mission parity, SCM v2 VM as the largest workstream | Missions out; SCM VM removed |
| RenderWare fixed-function fidelity, `bevy_pbr` disabled | Modern materials on `bevy_pbr` |
| M1–M2 "first pixels / whole world" as the experiment's answer | Same spine, but the answer is now *modern* first pixels and a 144 fps city |
| Period-accurate look as a success criterion | Asset identity + modernization, with an explicit fidelity ledger (§3) |
| Frame rate not addressed | Framerate independence as a hard requirement (D6) |
| AI mentioned as "largely script policy" | AI is in scope and is the top remaining risk (R1) |

---

## 12. Legal and distribution posture

- **No game assets in the repository, ever** — not in commits, releases, issues or CI artifacts.
  `gta-vc/` and the cooked-artifact directory (`vc-artifacts/`) must be gitignored, and `.gitignore`
  must exist before the first commit.
- The runtime reads a **user-owned install, read-only**, located via env var (`VC_GAME_ROOT`), never
  patched or replaced.
- **Cooked artifacts, generated mipmaps, re-encoded textures, navmesh data, impostors, upscaled
  texture packs and any exporter output are derived from the game and are equally non-distributable.**
  They are local build products.
- **Clean-room**: implement from public format documentation and observed bytes. Do not read
  `re3`/`reVC` (DMCA'd 2021-02-19; the original is blocked and returns HTTP 451) or GPL
  implementations (`rw-parser-rs`, OpenRW). MIT `librw` is acceptable as a *development-time
  oracle* only — never vendored, linked or shipped.
- **`reVC` is a black-box oracle only** (D7, decided 2026-10-26). The surviving public fork
  (`mrxenginner/reVC`) states in its own README that it is "the fully reversed source code for GTA
  VC" and that everything outside `FIX_BUGS` guards is "completely reversed code from original
  binaries". It licences itself as **MIT with a Proof-of-Usage condition**, which asks any automated
  system that ingests the repository to record a usage row and credit it — a non-permissive term, and
  one aimed squarely at tools like the agents working on this project. It is therefore both a
  derivative of Take-Two's code and outside our licence policy.
  - **Permitted**: building and running it — or the original `gta-vc.exe` — as an oracle, capturing
    what it does (§14.5).
  - **Forbidden**: opening its source, quoting it, porting logic from it, or reusing its identifiers
    and constants. A licence from the reVC authors cannot grant rights to code they do not own, so
    porting from it would make `vc-rs` an infringing derivative regardless of its stated licence.
  - Rationale for the record: the fastest route to behaviour parity would have been to port it, and we
    are deliberately declining that. The cost shows up as slower progress on AI and sim semantics
    (M5–M7), which §9 R1 already carries as the top risk.
- Neutral naming: repository `gta-vc-rust`, binary `vc-rs`; no Rockstar trademarks in the product
  name, no implied affiliation.

---

## 13. Dependencies

*This section is the shape of the dependency set. **§17 is the locked, version-pinned, licence-checked
table** and supersedes anything here that disagrees with it.*

| Need | Choice |
|---|---|
| Engine | `bevy 0.20` with `bevy_pbr` **enabled** (custom materials, clustered lighting, cascaded shadows, prepass, TAA), `bevy_render`, `bevy_asset`, `bevy_ui` + `bevy_text`, windowing via `bevy_winit` (exact feature spellings in §17) |
| GPU | wgpu/naga via `bevy_render` → D3D12 (Windows), Vulkan (Linux) |
| Math/buffers | `glam`, `bytemuck`, `half`, `encase` |
| Audio | `cpal` + `symphonia` (MP3/WAV) + `rtrb` for the realtime ring; own mixer |
| Physics queries | `parry3d` (Apache-2.0) for BVH + ray/point/contact queries; **our own** movement/vehicle response |
| Texture encode (cook) | **`ctt` 0.6.0** — BC1–BC7 plus mip chains, cook-only dependency (§17) |
| Hashing/cache | `blake3` for the content-addressed cook cache (hand-rolled store, no `cacache`) |
| Config / saves | `serde` + `toml` (settings), `postcard` (saves); cooked assets are `bytemuck` POD |
| Debug/profiling | Tracy/RenderDoc/PIX as external tools; in-process frame-budget reports |
| Stretch | AMD FidelityFX-style upscaling (MIT) — optional, decided at M10 (§21) |

**Explicitly not used:** `rw-parser-rs` (GPL-3.0), `libtxd` (MPL-2.0, unpublished), OpenRW/dff2gltf
(GPL-3.0), `re3`/`reVC` (blocked). GXT is small enough to own.

---

## 14. Verification

### 14.1 Asset layer
Census assertions (§4 numbers) as a build test; differential testing against MIT `librw` during
development with committed golden vectors; field-level round-trip equality.

### 14.2 Performance
Frame-time percentiles, hitch counters, draw calls, resident geometry, texture bytes and streaming
pressure reported in-process; per-milestone comparison against §6. A perf regression that breaches a
target fails the milestone.

### 14.3 Parity matrix
One file enumerating every subsystem × {not started, partial, ported, modernized, gap}, each row
citing the evidence that justifies it, plus a **gap register** and a **divergence list** (things we
deliberately changed: draw distance, mips, lighting, HUD, UI). "Ported" means the shipped data drives
it; "modernized" means the implementation changed but the behaviour is intended to match;
"authored" (e.g. `vc_director`) is labelled as ours.

### 14.4 Framerate independence (D6)
Replay a recorded input trace at 30/60/144/240 render fps and require the simulation trajectory to
match. This is the test that proves "works well on modern hardware" rather than merely "runs".

### 14.5 Reference behaviour and visuals

Two oracles, both driven strictly as black boxes:

- **The original `gta-vc.exe`** — ground truth for 1.0 behaviour and appearance at fixed camera poses.
- **`reVC`** — permitted as a *host* only (D7, §12): it runs on modern systems, has a debug camera,
  and is convenient for capturing behaviour traces that are awkward to record in the original.

Protocol: drive the oracle with recorded inputs and capture what it *does* — positions, velocities,
timings, AI decisions, screenshots — then diff our implementation against those traces. Screenshots
check *asset identity* (textures, colours, silhouettes, layout), explicitly **not** the 2,000-unit fog
or D3D8 shading, which we are replacing on purpose. Captures live in the ignored `vc-artifacts/` and
are never committed. Source is never read, quoted or ported (§12).

---

## 15. Appendix

### 15.1 Reproduction commands

```bash
cd gta-vc && find . -type f | sed 's/.*\.//' | tr 'A-Z' 'a-z' | sort | uniq -c | sort -rn
strings -n 6 gta-vc.exe | grep -i renderware          # → RW34Active/d3d8
python3 -c "import struct; d=open('models/gta3.dir','rb').read(); print(len(d)//32)"   # → 6043
find data/maps -name '*.ide' | wc -l ; find data/maps -name '*.ipl' | wc -l
find . -iname '*.col' | wc -l ; find Audio -name '*.adf' -printf '%f %s\n'
grep -c '^////' data/timecyc.dat                      # → 7 weather types
grep -n -A1 -iE '^\s*(cull|pick|path)\s*$' data/maps/*/*.ipl | head   # → empty blocks
```

Measurement scripts for §4.2/§4.5/§4.6 (chunk-tree walk, geometry counts, texture size accounting)
move into `xtask` at M0 so the numbers are re-derived rather than trusted.

### 15.2 Open probes (M0.5)

1. Install-version assertion — hash the census to pin 1.0 and detect 1.1/retail drift.
2. **Path graph cross-block links and coordinate scale** — the intra-block structure is decoded
   (§4.11); what remains is which field carries the links *between* blocks, and whether 1/16 is the
   right unit (R2).
3. **Spawn policy** — answered 2026-10-27 (§4.15): `pedgrp.dat` is per-zone ped groups, so peds need no
   script; `default.ide` carries car classes but no density, so only vehicle mix is authored.
4. **Framerate coupling** — closed 2026-10-27 (§4.17): the original does not run on the reference
   machine (black screen, audio only), the coupling is established from public evidence, and
   quantifying it would be reproducing the defect. D6 is the gate instead.
5. Full TXD raster-format histogram — how much of 182.6 MB is BC1 vs. needs decode.
6. **DFF geometry layout** — done 2026-10-27 (§4.12): decoded, byte-verified on all 6,505 geometries
   and gated by the census. What remains is the `0x50E` / `0x105` extension chunks, which are optional
   extras rather than part of the vertex data.
7. **Audio** — partly done 2026-10-27 (§4.7): `sfx.RAW` is 16-bit PCM, and the `.adf` container has
    two magics with MPEG audio inside. Still open: the `sfx.SDT` table layout, and the `.adf` container
    header itself (R6).
8. **Cull zones** — done 2026-10-27 (§4.14, §4.16): 631 boxes decoded, and the binary
    `Cullzone.dat` is confirmed to be the same zones in a different field order, so it is redundant.
9. **`models/coll/peds.col`** — scoped 2026-10-27, not decoded: it holds **8 records** (sizes 404 to
   1,360 bytes), not the ~30 that a marker count suggested, and no header/bounds/list combination —
   including every permutation of the four list orders and sphere strides of 16, 20, 24 and 28 —
   consumes a record exactly. The oracle route is closed (§10.1), so this needs either accounting
   forensics with more time or a ped-collision-specific approach at M5. It is 8 models out of 2,100+.
10. **Texture formats** — done 2026-10-27 (§4.6): all 11,997 descriptors are byte-exact, split
    10,824 BC1 / 1,149 BC2-or-BC3 / 24 paletted, with 895 carrying mip chains. What remains is
    separating DXT3 from DXT5, which the byte budget cannot do — a decoder-side question for M1.
11. **`timecyc.dat`** — done 2026-10-27 (§4.13): all 168 rows decode, with the column order confirmed
    against the file's own header. What remains is deciding how the 150–2,000 far clip curve maps onto
    the extended view distance of §5.2.
12. **Install identity** — done 2026-10-27: the census fingerprints the IMG directory (names, offsets
    and lengths, in order) and `--verify` fails when it changes, so a different build of the game
    cannot be measured silently. This install is `bf764075…`, GTA VC PC 1.00.000.
13. Bevy 0.20 custom-material + instancing hooks proven **in our own build** — the API shape is now
    verified from docs and shipped examples (§20), so the probe is a one-material smoke test, not a
    design question (R10).

---

## 16. Toolchain and environment (locked)

**Verified state of this machine, 2026-10-26:**

| Fact | Value | Consequence |
|---|---|---|
| OS | WSL2 on Windows 11 (`6.18.33.2-microsoft-standard-WSL2`) | two build environments, one repo |
| CPU / RAM | 12 cores; 15 GB reported to WSL | fine for the sim/AI budgets in §6 |
| GPU | **NVIDIA RTX 5060, 8 GB VRAM**, driver 610.74 | reference machine for §6; 8 GB is the constraint to design against |
| Vulkan in WSL | **no `nvidia_icd.json`** — only Mesa (`lvp` software, plus other vendors') | **no GPU rendering inside WSL** (R12) |
| `/dev/dxg` | present | D3D12 passthrough exists, but wgpu's D3D12 backend runs on Windows, not on Linux |
| Rust | **installed 2026-10-26** — WSL: `rustup` with 1.99.1 pinned in `rust-toolchain.toml`; Windows: rustup 1.29.1 installed via `winget` | M0 step 0 done (R13 closed) |
| Build tools | `gcc`, `g++`, `make`, `git`, `python3`, `pkg-config` and `cargo-deny` 0.20.2 present; **`cmake` and `clang` still missing** (installation needs a sudo password) | only the encoder backends and the `librw` oracle need them; `ctt`'s pure-Rust `bc7enc` backend avoids both (R17) |
| Disk | 96 GB free on `C:` (90% used), 954 GB free on the Linux ext4 root | build artifacts go on the Linux side |

**Locked decisions:**

1. **Toolchain**: `rustup` with **stable ≥ 1.97.1** (Bevy 0.20's MSRV, verified). Pinned in
   `rust-toolchain.toml` with `rustfmt` + `clippy`. No nightly. Workspace `edition = "2024"`.
2. **Rust is installed twice, deliberately**: WSL (cook tooling, parser tests, CI parity) and
   Windows (the runtime, D3D12). The repo lives on `C:` so both see the same tree.
3. **Never build from WSL into `/mnt/c`** — the 9p mount makes it several times slower. WSL sets
   `CARGO_TARGET_DIR=/home/mutex/.cache/vc-rs/target`; Windows uses the normal in-tree `target/`.
   Add this to the WSL setup notes and to `xtask` documentation.
4. **Run and measure on Windows native (D3D12)** (R12). WSL is for cook/parse/test/CI. If we later
   want Linux GPU runs, the NVIDIA Vulkan ICD must be installed — optional, not on the critical path.
   Mesa `lvp` (software Vulkan) is available for headless image tests only.
5. **Install `cmake` + `clang`** in WSL (encoder backends, and the MIT `librw` oracle for R7).
6. **Reference machine for performance = this box** (§6).

---

## 17. Dependencies (locked)

Versions verified against crates.io on 2026-10-26 (publish dates in parentheses). Policy: permissive
only (D4), enforced by `cargo-deny` (§18).

### Runtime

| Crate | Version | Licence | Use / notes |
|---|---|---|---|
| `bevy` | **0.20.0** (2026-10-08) | MIT OR Apache-2.0 | MSRV **1.97.1**, edition 2024. `default-features = false` with the explicit list below |
| `bevy_pbr` (feature) | 0.20.0 | MIT OR Apache-2.0 | custom materials, clustered lighting, cascaded shadows, prepass |
| `wgpu` | **30.0.1** (2026-08-22) | MIT OR Apache-2.0 | **transitive** via `bevy_render` (Bevy pins `wgpu = "30"`) — never a direct dependency; D3D12 on Windows, Vulkan on Linux |
| `glam` | **0.33.x** (0.33.12) | MIT OR Apache-2.0 | **must be 0.33** — Bevy 0.20 requires `^0.33.2`; 0.34 would duplicate (R15) |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | POD vertex/index/artifact structs |
| `half` | 2.7.1 | MIT OR Apache-2.0 | f16 colour/keyframe data |
| `encase` | 0.13.0 | **MIT-0** | WGSL uniform layout — add `MIT-0` to the deny allow-list or the build fails |
| `parry3d` | 0.31.1 (2026-09-18) | Apache-2.0 | BVH (QBVH) + ray/point/contact queries for collision lookups; **no physics engine** |
| `cpal` | 0.18.2 (2026-08-16) | Apache-2.0 | audio output. **Keep `asio` and `jack` features OFF** (Steinberg's account-gated SDK; libjack is LGPL) |
| `symphonia` | 0.6.1 (2026-08-13) | MPL-2.0 | MP3/WAV decode. `default-features = false, features = ["mp3","wav","pcm","adpcm","opt-simd"]` — `wav`/`pcm` are in default, **`mp3` is not**, and **`adpcm` is required** because 372 of the 1,120 loose WAVs are IMA ADPCM rather than PCM (§4.7) |
| `rtrb` | 0.4.1 (2026-10-25) | MIT OR Apache-2.0 | wait-free SPSC ring buffer for the realtime audio thread (`ringbuf` 0.5.3 is the ergonomic alternative) |
| `serde` + `toml` | 1.0.229 / 1.1.8 | MIT OR Apache-2.0 | settings and the config surface |
| `postcard` | 1.1.3 (2025-07-24) | MIT OR Apache-2.0 | save files (schema-driven, active) |
| `blake3` | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | cook cache keys; **no copyleft anywhere in its graph** |
| `memmap2` | 0.9.12 (2026-10-21) | MIT OR Apache-2.0 | read-only mapping of `gta3.img` / `sfx.RAW` |
| `dirs` | 7.0.0 (2026-09-05) | MIT OR Apache-2.0 | config/save locations (**replaces `directories` 6.0.0, whose repo is archived**) |
| `rfd` | 0.17.2 | MIT | install-folder picker. **Do not enable `gtk3`** (LGPL system lib); keep defaults |
| `anyhow` / `thiserror` | 1.0.104 / 2.0.21 | MIT OR Apache-2.0 | `anyhow` at app/xtask edges, `thiserror` for typed library errors |
| `tracing` + `tracing-subscriber` | 0.1.44 / 0.3.23 | MIT | diagnostics; Bevy's `LogPlugin` is the front end |
| `rayon` | 1.12.0 | MIT OR Apache-2.0 | cook-time and offline parallelism (runtime uses Bevy's task pools) |

### Cook / tooling only (never linked into the runtime binary)

| Crate | Version | Licence | Use / notes |
|---|---|---|---|
| `ctt` | **0.6.0** (2026-09-27) | MIT OR Apache-2.0 OR Zlib (sub-crates add AND-backend) | **the texture encoder**: BC1/BC2/BC3/BC4/BC5/BC6H/**BC7** plus mip-chain generation and DDS/KTX2 containers. Maintained by a wgpu maintainer; runs its own permissive-only `cargo deny` |
| `clap` | 4.6.7 | MIT OR Apache-2.0 | `xtask` CLI |
| `png` | 0.18.1 | MIT OR Apache-2.0 | debug/`export-gltf` images only (`image` 0.25.10 is the heavier alternative) |

### Dev

`criterion` 0.8.2, `proptest` 1.12.0, `insta` 1.49.0, **`cargo-deny` 0.20.2** (licence/bans/advisory
policy in `deny.toml`).

### Bevy feature list (exact names, verified)

```toml
bevy = { version = "0.20", default-features = false, features = [
  "bevy_asset", "bevy_render", "bevy_core_pipeline", "bevy_pbr",
  "bevy_post_process", "bevy_anti_alias",          # bloom/DoF/CA · TAA
  "bevy_winit", "x11", "wayland",                   # Linux windowing
  "bevy_ui", "bevy_ui_render", "bevy_text",
  "bevy_sprite", "bevy_gilrs", "multi_threaded",
  "png",                                            # debug export only
] }
```

Notes that matter: **there is no BC cargo feature** — BC support is a *runtime device* capability
(`CompressedImageFormats::BC`, from `wgpu::Features::TEXTURE_COMPRESSION_BC`). Our cooked-texture
loader builds `Image`s directly with `TextureFormat::Bc1RgbaUnorm`/`Bc3RgbaUnorm`/`Bc7RgbaUnorm`, its
own mip chain and an anisotropic sampler; `dds`/`ktx2` stay off unless we want them for debugging.
TAA lives in `bevy_anti_alias`, bloom in `bevy_post_process`, tonemapping in `bevy_core_pipeline` —
they are three separate features, not one.

### Rejected, with reasons

| Rejected | Reason | Substitute |
|---|---|---|
| `nvtt_rs` | MIT bindings, but links a **separately installed, account-gated NVIDIA NVTT 3 SDK** with no published OSS licence → "unclear licence" under D4 | `ctt` |
| `texpresso` | Licence is fine (MIT, verified), but **BC7 is unimplemented** and there is no mip generation — insufficient alone | `ctt` (keep only as a no-C++ fallback) |
| `basis-universal` / `basisu` | Stale bindings (2023) and architecturally wrong (ETC1S/UASTC + runtime transcode, not an offline BC cook) | `ctt` |
| `rgbcx` | MPL-2.0 but 5 weeks old, zero stars, one author, **MSRV 1.98.1** | `ctt` |
| `directories` | Repo archived (2025-01-12) | `dirs` 7 |
| `bincode` 3.0 | GitHub repo archived, development moved to sourcehut, ~0.16% of 2.0.1's adoption | `postcard` |
| `rkyv` (for cooked assets) | MIT and maintained, but 0.7→0.8 was a hard format break and 0.9 is pending — too much on-disk-format risk; zero-copy also only pays off for pointer-rich graphs | `bytemuck` POD + versioned header |
| `cacache` | Only real content-addressed cache crate, but no release in ~2 years | hand-rolled ~200 lines over `blake3` + atomic rename |
| `rapier3d`, `avian3d` | Full solvers we don't want; VC's feel is arcade and hand-derived | `parry3d` queries only |
| Meshlets / mesh shaders | Bevy's meshlet path is **Vulkan/Metal only**, forbids MSAA, and is opaque-only — unusable on our D3D12 target | plain meshes + indirect draws |
| `rw-parser-rs`, OpenRW | GPL-3.0 | our own parsers (§12) |
| `re3`/`reVC` (any fork) | decompiled game code; the surviving fork is MIT with a Proof-of-Usage condition aimed at automated assistants | never read or vendored; run as a black-box oracle only (D7, §12) |
| `bevy_gltf` | Not used — our assets cook into our own format | — |
| `bevy_kira_audio` | We own the mixer (§8.5) | — |

### `cargo-deny` allow-list (must include these or known-good crates fail)

`Apache-2.0`, `MIT`, **`MIT-0`** (`encase`), `BSD-3-Clause`, `Zlib`, `MPL-2.0` (`symphonia`),
**`CC0-1.0`** and **`Apache-2.0 WITH LLVM-exception`** (`blake3`), `ISC`, `Unicode-3.0`, `Unlicense`,
`NCSA`. Deny GPL-2/3, AGPL, LGPL. `multiple-versions = "warn"` (the encoder backends will overlap).
Documented blind spot we must cover by review: `cargo-deny` cannot detect a crate whose declared
SPDX differs from what it actually links (R17).

---

## 18. Repository scaffolding and process (locked)

1. **Project licence: dual `MIT OR Apache-2.0`** (`LICENSE-MIT`, `LICENSE-APACHE`,
   `license = "MIT OR Apache-2.0"` in the workspace manifest). Rationale: maximum compatibility for a
   clean-room project, matches the permissive set we already accept, and keeps a GPL boundary intact.
2. **`git init` plus `.gitignore` as the very first commit**, before any code: `/target`, `gta-vc/`,
   `vc-artifacts/`, `.env`, `*.env`, `context/`. (This directory is not a git repo today.)
3. Layout: `crates/*` + `xtask`, `edition = "2024"`, `resolver = "3"`, workspace-pinned dependency
   versions, `[workspace.lints]` with `clippy` warnings denied and `unsafe_code = "deny"` outside
   narrowly-documented FFI-free exceptions.
4. Tooling config committed: `rust-toolchain.toml`, `rustfmt.toml`, `deny.toml`, plus Cargo profiles
   `dev` (fast deps), `release` (LTO), `perf` (debug symbols + frame pointers, for the §6 harness).
5. **CI (Linux, CPU-only)**: `cargo fmt --check`, `clippy -D warnings`, `cargo test` on synthetic
   fixtures, `cargo deny check`, build. **No game data, ever** (R16).
6. **Test policy.** Parser unit tests use *synthetic* fixtures committed to the repo. Anything that
   needs the real install (`xtask census --verify`, the `librw` differential oracle, reference-frame
   captures) is **local-only**, reads `VC_GAME_ROOT`, and writes to the ignored `vc-artifacts/`.
   A test that cannot run in CI is still a first-class test — it just cannot ship its inputs.
7. Naming: binary `vc-rs`, crates `vc_*`, env var `VC_GAME_ROOT` (defaults to `./gta-vc` for this
   checkout), artifacts `vc-artifacts/`.

---

## 19. M0 start-work checklist

Ordered; steps 0–2 are prerequisites for everything else.

**Progress, 2026-10-26:** **M0 is complete.** Steps 0–6 are done: Rust 1.99.1 pinned in WSL with
rustup on Windows, `cargo-deny` installed, the repository initialised with the game install ignored,
the workspace passing `cargo clippy -- -D warnings` / tests / `cargo deny check`, and the verifier in
place — `vc_transport` (install discovery, IMG v1 archive), `rw_stream` (section tree, shared by the
format crates), `vc_dff`, `vc_txd`, `vc_col`, `vc_ipl`, `vc_ide`, plus `cargo xtask census --verify`
and `cargo xtask dump`.

Running the verifier against the install **corrected four measurements from the first pass** (§4.10)
and produced the totals §4 now quotes. It also surfaced one real parser bug on the way (a nesting
counter in the section iterator) and one genuine unknown (`models/coll/peds.col`).

Next: the M0.5 probes (§15.2), then M1 — first pixels.
Remaining blocker: `cmake`/`clang` (needs a sudo password; only the encoder backends and the `librw`
oracle need them).

| # | Task | Done when |
|---|---|---|
| 0 | Install `rustup` (WSL + Windows), verify `rustc ≥ 1.97.1`, `apt-get install cmake clang` in WSL, commit `rust-toolchain.toml` | `cargo --version` works on both sides; `cmake`/`clang` resolve |
| 1 | `git init`; `.gitignore` (before anything else); dual licence files; `README.md` pointing at this plan | `git status` shows only intended files; `gta-vc/` is invisible to git |
| 2 | Workspace skeleton: `vc_transport` lib, `xtask` bin, `deny.toml`, workspace lints, profiles | `cargo build`, `cargo clippy -D warnings`, `cargo deny check` all pass on the empty workspace — this validates the whole licence/feature plan before real code exists |
| 3 | `xtask census` — walk the install via `VC_GAME_ROOT`, tally IMG/DIR entries, per-format counts, geometry/triangle totals | `xtask census --verify` reproduces §4.1–4.3 and fails loudly on mismatch |
| 4 | `rw_stream` chunk-tree reader + `xtask dump <file>`; version-tolerant parsing (`0x0C02FFFF` DFF / `0x1003FFFF` TXD) | `xtask dump` prints a correct tree for a DFF and a TXD; synthetic-fixture unit tests pass |
| 5 | `vc_transport` IMG v1 reader (`memmap2`), sector math, duplicate-name handling | 6,043 entries resolved; duplicates keyed by offset, not name |
| 6 | `vc_txd` + `vc_dff` skeleton: dictionary/sub-chunk walk, texture descriptor + level sizes, geometry counts | census covers textures and geometry; the TXD-format histogram probe (§15.2 #5) can run |
| 7 | Probe harness: one evidence file per probe under `vc-artifacts/probes/`, with a documented method | The ten §15.2 probes each have a place to land |

Then M0.5 (§10) runs the probes, and M1 starts on the first rendered area.

---

## 20. Corrections and refinements to §§5/6/8 (verified against Bevy 0.20)

Verified against Bevy 0.20 docs, the 0.19→0.20 migration guide and shipped examples on 2026-10-26.
Where this contradicts earlier prose, **this section wins**.

| Topic | Earlier assumption | Verified reality | Effect on the plan |
|---|---|---|---|
| GPU-driven rendering | "optional stretch; we write our own culling pass" | Bevy 0.20 runs **GPU preprocessing + indirect draws by default** (`GpuPreprocessingMode::Culling`; `NoIndirectDrawing` opts out) | Delete that engine work from M2; keep a perf harness check that it is actually active |
| Occlusion culling | "implement hi-Z ourselves" | **Built in**: `OcclusionCulling` component + `OcclusionCullingPlugin`, two-phase culling with a hierarchical Z-buffer, **no baking**; requires `DepthPrepass`; **incompatible with deferred shading**; **disabled by default** and documented as "only enable if you measure it to be a speedup" | Use the prepass + forward path, enable it, and measure per area; `Cullzone.dat` becomes a streaming hint only |
| Instancing | "Bevy batches; verify later" | Automatic instancing **works with any `Material`**, but the batch key includes **`AssetId<Mesh>`** — different mesh assets never share a batch; `NoAutomaticBatching` exists to opt out | Cook **dedupes geometry** so IPL instances share mesh handles; assert batch counts in the M2 harness (R14) |
| Meshlets / mesh shaders | considered as a modern option | Bevy's meshlet path is **Vulkan/Metal only**, forbids MSAA, opaque-only; D3D12 target can't use it | Excluded (§17). Revisit only if we add a Vulkan-only renderer |
| Mipmaps | "generate at load or in the cook" | Bevy does **not** generate mips for loaded images automatically (only via the asset processor or explicit `MipGenerationJobs`) | Mip generation is a **cook** responsibility — which is also where the BC1-vs-BC7 decision is made |
| Vertex attributes | "2 UV sets + day/night prelit" | `Mesh::ATTRIBUTE_UV_1` exists (dual texture ✓), but there is **exactly one colour attribute** (`ATTRIBUTE_COLOR`) | Add one custom `MeshVertexAttribute` (fixed id) for the second prelit set |
| Shadows | "cascaded sun + spot shadows" | `DirectionalLightShadowMap` sizes **directional and spot** maps; there is **no** `SpotLightShadowMap`; `SpotLight.shadows_enabled`; cascades via `CascadeShadowConfigBuilder`; `ContactShadows` and per-camera `ShadowFilteringMethod` exist | Tune cascade distances for the extended far plane (§5.2); use one shadow-map size for both light types |
| Post-processing | "HDR, TAA, MSAA" | Feature split: TAA → `bevy_anti_alias`; bloom/DoF/motion blur → `bevy_post_process`; tonemapping → `bevy_core_pipeline` | Three features, not one (§17) |
| WESL | "write WESL" | `.wesl` files; `import path::to::module;` with semicolons at the top; `@if(...)`; `constants::MATERIAL_BIND_GROUP`; module paths mirror crate paths (`bevy_pbr::render::forward_io`, `bevy_pbr::prepass::utils`); `naga_oil` gone, GLSL removed | Adopt these conventions from the first shader; any pre-0.20 Bevy shader snippet found online is stale |
| Custom material API | — | All 17 `Material` methods are **provided** (none required); `specialize` + `MeshVertexBufferLayoutRef::get_layout` is the documented path; `MaterialPlugin::<T>` + `Mesh3d`/`MeshMaterial3d` | M1 smoke test is a small, low-risk step, not a design question |
| Texture encoder | `texpresso`/`nvtt_rs` | `nvtt_rs` links an account-gated SDK (rejected); `texpresso` has no BC7 and no mips | **`ctt` 0.6.0** is the encoder (§17) |

Two invariants worth writing into code comments once M1 lands: (a) *batching needs shared mesh
handles*; (b) *occlusion culling needs a depth prepass and forward shading, and must be measured*.

---

## 21. Deferred by design (deliberately not decided yet)

These are **not gaps** — each has an owner milestone and a stated criterion. Listed so nobody
mistakes an open design choice for an oversight.

| Decision | Decided at | Criterion |
|---|---|---|
| Water reflection technique (planar vs. screen-space vs. cubemap probe) | M2 | whichever holds 144 fps at 1440p with water visible in the densest waterfront view |
| BC1 vs. BC7 per texture class | M1 (cook) | visual diff on foliage/fences/glass vs. the 1.5 GB VRAM budget (§6) |
| Whether occlusion culling is net-positive per area | M2 | measured; Bevy's own docs say enable only if it is a speedup |
| AI LOD thresholds and population caps | M6 | tick-cost histogram against the ≤4 ms budget |
| Save-file schema and mod-override layout | M9 | postcard schema is small; mods are loose-file overrides mirroring the cooked tree |
| Upscaling (FSR-style) | M10 (stretch) | only if 1440p/144 fps misses on the reference machine |
| Linux GPU support (install NVIDIA Vulkan ICD) | post-M10 (optional) | Windows/D3D12 is the supported run target (R12) |
| Meshlets / mesh shaders | only if a Vulkan-only renderer is ever added | unavailable on D3D12 (§20) |
| Radar blip art and HUD icon set | M9 | rebuilt from `hud.txd`/`MISC.TXD`, measured against the original's readability |
| Cooked-asset hot reload for the dev loop | M1/M2 (dev-only QoL) | a file watcher (`notify`) re-cooks one area on save; never enabled in shipping builds |