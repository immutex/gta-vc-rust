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
| D6 | **Framerate independence is a hard requirement** | VC is frame-coupled and needs its ~30 fps limiter; every ported system must behave identically at 30/60/144/240 fps render rates. Verified by test, not by hope (§14.4). |

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
| Map layout, 9,114 instances, 3,511 object defs, 314 time objects | Draw distance (FarClp 2,000 → 5,000–8,000 + atmosphere) | Vehicle handling (`handling.cfg` values used verbatim) |
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
| Models / geometries | 4,613 / 6,506 |
| **Triangles** | **2,207,065** |
| **Vertices** | **2,819,169** |
| Skinned meshes | **0** |

2.2 M triangles is a light load for a modern GPU — the modern-HW budget problem is **not** geometry,
it is draw-call count, overdraw, streaming and CPU simulation. Skinning being absent (ped/vehicle
bodies are rigid atomic hierarchies animated by per-frame matrices) removes another whole subsystem.

### 4.3 World data — what is populated, and what is empty

| Source | Content | Status |
|---|---|---|
| IPL `inst` | **9,114 instance lines** (29 area files) | populated |
| IDE `objs` / `tobj` / `2dfx` | 3,511 object defs / 314 time-controlled objects / **985 2dfx entries** | populated — 2dfx are lights/coronas/particles, i.e. **modernization material** (§5.7) |
| IPL `cull` | cull zones | **empty in all 30 files** — cull zones live in the binary `data/Cullzone.dat` (155 KB, NUL-leading, non-text) |
| IPL `pick` | pickups | **empty in all 30 files** — pickups are script-created ⇒ `vc_director` authors them |
| IPL `path`, IDE `path` | path nodes | **empty in all 61 blocks** — see R2 |
| `data/paths/` | only `flight.dat` (336 nodes), `flight2.dat` (158), `flight3.dat` (485), `spath0.dat` (79) | flight/script paths only; **no ground ped/vehicle graph** |
| `data/timecyc.dat` | **7 weather types × 24 hours**, 23 columns (ambient, ambient-object, directional, sky top/bottom, sun core/corona, shadow, light-shadow, pole-shadow, **FarClp**, fog, `LightOnGround`, 3 cloud colours, blur RGB, water RGBA) | populated — the master table for the modern atmosphere/sky model |
| COL | 64 files (30 IMG + 34 loose), **4,272 records** | populated |
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
│   │                 ambient/diffuse/specular RGBA
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

120 sampled textures across 120 TXDs: **64 gave a byte-exact `width × height / 2` payload** — i.e.
**single-level DXT1/BC1**, **no mip chain** (`256²`→32,768 B; `128²`→8,192 B; `64²`→2,048 B;
`32²`→512 B; names confirm dimensions). The remainder need the full descriptor walk (M0.5).

Two modernization consequences, both high-value and cheap:

1. Block copy to BC1/BC3 — no decode, no recompression for the base level.
2. **The original has no mipmaps.** Generating mip chains in the cook plus anisotropic filtering is
   the single cheapest, most visible image-quality upgrade available (§5.3).

### 4.7 Audio

- `sfx.RAW` (340 MB) + `sfx.SDT` (199 KB offset/description table) — the SFX bank; codec to confirm.
- 1,120 loose WAV (PCM) + 90 MP3 (ambience/cutscene-adjacent music) — decodable today.
- **9 `.adf` radio stations**, 41–74 MB each (~490 MB), high-entropy, proprietary, undocumented →
  the one item that may end as a documented gap (R6).
- `mss/mss32.dll`, `.m3d`, `Reverb3.flt` are Miles Sound System artefacts; we do not emulate them.

### 4.8 Correction from the first draft

`data/main.sc` and `data/debug.sc` are **leftover developer test scripts** ("Test script for
Industrial level"), not the game's mission sources. The first draft mislabelled them as shipped
Mission-Builder sources. With missions out of scope this only matters as a caution: there is **no
readable source** for the shipped mission logic, which is consistent with leaving it alone.

### 4.9 RenderWare stamps

DFF `0x0C02FFFF`; TXD `0x1003FFFF` — mixed within one install, so parsers must branch on the stamp
rather than pinning a version.

---

## 5. Modernization plan

The heart of this revision: what the original does, what we do instead, and what it buys.

| # | Original (2002) | Modern approach | Gain |
|---|---|---|---|
| 5.1 | **Logic coupled to frame rate**; VC needs its ~30 fps limiter, and high frame rates break timers/physics | **Fixed-step simulation** (60 Hz) fully decoupled from rendering; render interpolates transforms; every timer expressed in ticks; input sampled per frame, consumed per tick | Stable behaviour at any refresh rate — the prerequisite for "work well on modern hardware" (D6) |
| 5.2 | **`FarClp` = 2,000 units**, fog used to hide the cut | Extended far plane (**5,000–8,000**), LOD chain (existing `lod*.dff`), **distant-city impostors/HLOD baked in the cook**, modern atmospheric fog driven by timecyc's fog/sky columns | Vice City reads as a city instead of a fog bank; the biggest visual change |
| 5.3 | **No mipmaps**; point/linear filtering; DXT1 | **Generate mip chains in the cook**, anisotropic filtering, trilinear; BC7 for alpha-heavy textures (2× VRAM, ~366 MB — affordable) | Kills 2002 shimmer on roads, fences, foliage, railings |
| 5.4 | **Immediate-mode per-object draws**, D3D8 | Bevy 0.20 already batches repeated mesh+material pairs and **runs GPU preprocessing + indirect draws by default**; the cook dedupes geometry so instances share mesh handles; our CPU work is limited to per-cell streaming/LOD decisions | Draw calls collapse to hundreds — and this is configuration, not engine work (§20) |
| 5.5 | Cull zones + occluder boxes, BSP-less CPU culling | **Bevy 0.20 ships two-phase GPU occlusion culling** (`OcclusionCulling` + depth prepass, hierarchical Z-buffer, no baking); enable it and measure; `Cullzone.dat` retained as a coarse streaming hint | Long draw distance without paying for it — again configuration rather than engine work (§20) |
| 5.6 | Fixed-function shading; no shadows beyond a shadow blob | Custom `Material`s on Bevy's pipeline: cascaded sun shadows, spot shadows for headlights, alpha-test/dual-texture preserved, `AlphaMode`/blend from RW state | Modern shading that keeps the art direction |
| 5.7 | Prelit vertex colours + a handful of dynamic lights | Prelit = baked indirect term (unchanged), **plus** dynamic sun/sky from timecyc and **clustered local lights driven by the 985 2dfx entries** (street lamps, neon, coronas become real lights) | Night-time Vice City modernized — the neon city finally lights itself |
| 5.8 | Flat water plane, cheap animation | Modern water: animated normals, reflection/refraction, depth tint, timecyc `WaterRGBA`; polygons still from `water.dat`/`WATERPRO.DAT` | Modern look, same geography |
| 5.9 | Synchronous IMG streaming, TXD-from-archive | **Async cooked-artifact streaming** (background threads, residency budgets, cell/frustum priority); cooked artifacts are compile-time-optimized (mips, BCn, quantized vertex streams, GPU-ready layouts) | No hitches, faster loads, controlled VRAM |
| 5.10 | All AI ticked uniformly | **AI LOD**: tick rates by distance/visibility; budgeted ECS job scheduler; cheap steering for distant peds | Hundreds of active peds without CPU spikes |
| 5.11 | 2002 node graph (not shipped — R2) | **Offline-generated nav**: navmesh/node graph built from cooked collision, hierarchical A* + flow fields + path cache | Modern pathfinding covering the whole map, no dependency on missing data |
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
- Navigation: navmesh/node graph generated offline from collision (R2), hierarchical A*, flow fields
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
| R2 | **Ground nav graph is not in the shipped data** — all 61 `path` blocks are empty; `data/paths/` holds only flight/script paths | **High** | `xtask navbuild`: generate navmesh/node graph offline from cooked collision. Also probe M0.5 for an undiscovered source (binary `Cullzone.dat` is a candidate). Generation is arguably the better modern answer either way. |
| R3 | **Framerate-dependence retrofit** — the original's logic/timers are frame-coupled; VC is known to require its ~30 fps limiter | **High** | M0.5 probe to confirm timing sources; fixed-step core from M3 onward; D6 test (identical trajectory across render rates) as a hard gate, not a nice-to-have. |
| R4 | **Spawn/ambient policy was partly script-driven** (IPL `pick` and `cull` blocks are empty; pickups are script-created) | Medium | `vc_director` authors it; M0.5 estimates how much. Recorded in the fidelity ledger as authored, not ported. |
| R5 | **Modern lighting can make 2002 art look wrong** (over-lit, plastic) | Medium | Prelit colours remain the base term; lighting is additive; keep a "period" toggle for A/B comparison against the original. |
| R6 | **`.adf` radio** — proprietary, high-entropy, ~490 MB, no public codec | Medium | Time-boxed M0.5 spike. Failure ⇒ documented gap (iw4L precedent: report gaps, never synthesise). |
| R7 | **DFF vertex encoding details** (quantisation, attribute order, `0x50E`/`0x105`, dual UV/prelit sets) | Medium | Differential testing against MIT `librw` as a development-only oracle; golden vectors in CI. |
| R8 | **Offline cook vs. semantic loss** | Medium | Cooked formats store material state words and vertex streams verbatim; glTF is a debug lens only. |
| R9 | **Legal exposure** — Take-Two DMCA'd comparable projects (re3/reVC blocked, HTTP 451) | Medium | §12. |
| R10 | **Bevy 0.20 specifics**: WESL shader syntax replaced `#import`/`#ifdef`; instancing/batching assumptions and `bevy_pbr` custom-material hooks need confirming at M1 | Low–medium | Verify hooks at M1 with one material before scaling out; shaders authored in WESL from day one. |
| R11 | **Impostor/HLOD authoring is new content work** — no distant-city LODs ship beyond the `lod*.dff` chain | Low–medium | Cook-time generation from existing LOD geometry; measure against 5.2's target. |
| R12 | **No hardware GPU inside WSL** — the WSL driver exposes CUDA/D3D12 bits but no NVIDIA Vulkan ICD (only Mesa software `lvp` and other vendors' drivers) | **High** (blocks "run it and see") | Verified 2026-10-26. Rendering runs and is measured **on Windows native (D3D12)**; WSL stays the CPU-side cook/test/CI environment. `lvp` software Vulkan is available for headless image tests, but is not representative for perf. §16 |
| R13 | **No Rust toolchain installed anywhere** (WSL `~/.cargo` empty; Windows side has only a stray `cargo-nextest.exe`), and Bevy 0.20 needs **Rust ≥ 1.97.1** | **High** (blocks everything) | First M0 action: install rustup on both sides, pin `rust-toolchain.toml`, verify ≥ 1.97.1. §16, §19 |
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
| **M0** | **Recon + verifier** | `vc_transport` + `rw_stream` + `cargo xtask census` reproduces §4 exactly (6,043 IMG entries with 11 duplicate names; 4,617/1,368/30/28; 4,613 models; 6,506 geometries; 2,207,065 triangles; 9,114 instances; 3,511 objs / 314 tobj / 985 2dfx; 64 COL files / 4,272 records; install pinned to 1.0). Mismatch fails the build. |
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
  `re3`/`reVC` (DMCA'd 2021-02-19; blocked, HTTP 451) or GPL implementations (`rw-parser-rs`,
  OpenRW). MIT `librw` is acceptable as a *development-time oracle* only — never vendored, linked or
  shipped.
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

### 14.5 Visual reference
Reference captures from the original at fixed camera poses: used to check *asset identity* (textures,
colours, silhouettes, layout) — explicitly **not** to reproduce 2,000-unit fog or D3D8 shading.

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
2. Ground nav-graph source, or confirmation that we generate it (R2).
3. How much ambient/spawn policy was script-driven (R4).
4. Framerate coupling: confirm which subsystems are frame-stepped in the original (R3).
5. Full TXD raster-format histogram — how much of 182.6 MB is BC1 vs. needs decode.
6. DFF vertex attribute encodings and the meaning of `0x50E` / `0x105` (R7).
7. `sfx.RAW` codec (PCM vs. ADPCM) and the `.adf` container verdict (R6).
8. `Cullzone.dat` binary layout.
9. `timecyc.dat` column semantics per weather, including `LightOnGround` and the cloud/blur columns.
10. Bevy 0.20 custom-material + instancing hooks proven **in our own build** — the API shape is now
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
| Rust | **not installed in WSL** (`~/.cargo` empty) and **not installed on Windows** (only a stray `cargo-nextest.exe` in `C:\Users\mutex\.cargo\bin`) | blocking; M0 step 0 (R13) |
| Build tools | `gcc`, `g++`, `make`, `git`, `python3`, `pkg-config` present; **`cmake` and `clang` missing** | install for the encoder backends (R17) |
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
| `symphonia` | 0.6.1 (2026-08-13) | MPL-2.0 | MP3/WAV decode. `default-features = false, features = ["mp3","wav","pcm","opt-simd"]` — `wav`/`pcm` are in default, **`mp3` is not** |
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
| `rw-parser-rs`, OpenRW, `re3`/`reVC` | GPL-3.0 / DMCA'd | our own parsers (§12) |
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