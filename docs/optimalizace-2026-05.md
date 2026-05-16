# Optimalizace core, 2026-05

Souhrn implementační vlny vycházející z analýzy 2026-05-11. Z původní sady 11 detailních plánů (`docs/plan-*.md`) jich 9 dolétlo do produkce, 1 zůstal částečně neimplementován ze záměru a 1 byl zamítnut po měření. Detailní plány byly smazány po sloučení do tohoto dokumentu 2026-05-16 — historie zůstává v `git log`, klíčové commity jsou uvedené níže u každé položky.

Tento dokument je archiv. Cíl: kdokoli (lidský čtenář, příští agent) musí ze souhrnu pochopit, co se udělalo, proč a co se vědomě neudělalo, aniž by musel rekonstruovat zaniklé plánové dokumenty.

## Co se implementovalo

### apply-outflow-splice

Commit `26d7d53` (2026-05-12). Rope-based merge (`MergeSegment` enum + thread-local scratch buffer) v `crates/aenternis-core/src/tick.rs:589-861` nahradil per-insert `Vec::splice` v `apply_outflow`. Bit-parita pinována baseline hash testem `tests/apply_outflow_bit_parity.rs`; 13 dominance/intrusion testů v `tests/tick_dominance.rs`. Žádné `unsafe`, žádná divergence od původního návrhu.

### cell-soa (Etapa 1)

Commit `e5fcaaf` (2026-05-12). `crates/aenternis-core/src/world/cells.rs` zavedl slot-indexed `Cells` wrapper kolem `FxHashMap<Coord, Cell>` — `coord_to_slot: FxHashMap<Coord, usize>` + `slots: Vec<Option<(Coord, Cell)>>` + `free_slots: Vec<usize>` s LIFO recyklací. Marginální win na `tick_step`, žádná regrese, bit-parita drží. **Etapa 2 (full SoA Cell split) vědomě odložena — viz "Co se neimplementovalo".**

### clamp-dedup

Commit `3737536` (2026-05-11). `apportion_with_shuffle` v novém modulu `crates/aenternis-core/src/apportion.rs` (Largest-Remainder + Fisher-Yates + stable sort); `combined_clamped` v `tick.rs:352-370` i `proportional_clamp` v `cell.rs:212-228` na něj delegují. RNG-domény oddělené (`COMBINED_CLAMPED_RNG_DOMAIN = 1`, `PROPORTIONAL_CLAMP_RNG_DOMAIN = 2`), fast-path konsolidovaná uvnitř `apportion_with_shuffle`. Žádná divergence.

### collect-outflow-slice

Commit `86735ea` (2026-05-11). `extend_from_slice` v `collect_outflow_into` (`tick.rs:272-285`) nahradil push-by-push loop s runtime modulem. **Perf hypotéza se nepotvrdila** (všechny scénáře `p > 0.05`), ale refaktor ponechán pro čitelnost — `extend_from_slice` komunikuje intent líp než ruční `(ptr + k) % mem_size` loop. Bit-identita zachována.

### deterministicky-round + isotropie

Vznikly jako dva propojené plány; implementačně tvoří jednu změnu ve třech commitech: doménový RNG salt `d529ccb` (2026-05-10), centralizace algoritmu v `apportion.rs` `3737536` (2026-05-11), tick-level isotropy testy `56c8e10` (2026-05-13). Largest-Remainder + Fisher-Yates + stable sort produkují statisticky izotropní výstup — pinováno testem `per_direction_inflow_is_balanced_over_many_ticks` v `tests/tick_isotropy.rs` a property testem v `tests/tick_combined_clamped_contracts.rs`.

**Divergence:** Původní kontrakt v plánu zněl jako strict per-call equivariance `π(combined_clamped(π⁻¹(...))) == combined_clamped(...)`. Praxe vyšla jako **statistická izotropie** přes mnoho seedů — což je matematicky nutné u shuffle tie-breaku a operationálně dostačující. Kompromis dokumentován v `apportion.rs:35-45` a `tests/tick_combined_clamped_contracts.rs:10-37`. Bit-parita vůči JS prototypu 9-B byla u této změny vědomě uvolněna; plošný cleanup JS-ism komentářů zůstává opportunistický, ne big-bang.

### par-or-seq

Commit `204a091` (2026-05-12). `par_or_seq_iter_mut!` makro v novém modulu `crates/aenternis-core/src/parallel.rs` s prahem `PAR_THRESHOLD = 8_192`; všech 6 callsites v `tick.rs` (`compute_natural_rates`, `refresh_neighbor_energies`, `collect_outflow_into`, `lay_out_pointers`, `apply_outflow`, `cpu_phase`) přepsáno. Sjednoceno 6× duplikované `#[cfg(target_arch = "wasm32")]` větve.

**Divergence:** Původně navržená generická `pub(crate) fn par_or_seq_iter_mut<K, V, F>` přinesla ~40 % LLVM regrese na parallel path (function-call overhead defeatoval size-check optimalizaci). Finální implementace je `macro_rules!` (zachová closure capture context bez fn boundary). Důvod dokumentován přímo v `parallel.rs:17-28`.

### snapshot-encoding

Bench skeleton commit `fdf2f31` (2026-05-12), implementace ve více předcházejících commitech. `encode_snapshot_frame_into` / `encode_cell_detail_frame_into` v `crates/aenternis-server/src/protocol.rs`, persistentní buffery v `WorldActor` (`snapshot_buf: Vec<u32>`, `encoded_buf: Vec<u8>`), `broadcast_snapshot(&mut self)` s `Arc<[u8]>` payloadem a receiver-count skipem. WASM `World::cells_snapshot(&mut self)` reusuje `snapshot_buf` napříč ticky. Žádná divergence — wire format beze změny, `bytemuck` nepřidáno (Cesta B: `resize` + `copy_from_slice`, žádné `unsafe`).

### sorted-index-bbox

Commit `1c661e2` (2026-05-11). Čtyři pole na `SparseWorld` (`sorted_cache`, `sorted_dirty`, `bbox_cache`, `bbox_dirty`), mutátory (`insert`, `remove`, `get_or_alloc`, `gc_empty`) hookují dirty flagy a inkrementální bbox-extend; `rebuild_indices_if_dirty()` provádí lazy rebuild na začátku každého `tick::step` / `step_diffusion` **před** inkrementem `tick`. Návratový typ `SparseWorld::sorted_iter` uvolněn z `std::vec::IntoIter` na `impl Iterator`. 8 testů v `tests/sparse_world.rs`, žádná divergence.

### wasm-zerocopy-threads

Dvoufázová.

**Part B (threads, 2026-05-13 až 2026-05-16, commity `7a12a0f`, `537ef53`, `29d3de9`, `6735ddd`, `9469eda`, `93a7602`, `e1a08bb`, `cc493d2`, `fa0eeec`, `c83fb56`):** Feature `wasm-threads` v `aenternis-wasm` i `aenternis-core` (`Cargo.toml:34, 37`), `par_or_seq_iter_mut!` cestuje přes feature gate, `scripts/build-wasm.sh` s `--features wasm-threads` + nightly toolchain (kvůli atomics build flagům), COOP/COEP hlavičky ve `vite.config.ts:22-25` plus service-worker shim (`coi-serviceworker.js`) pro GitHub Pages, `initThreadPool(navigator.hardwareConcurrency)` ve `web/worker.ts:46-64` s feature detection (`crossOriginIsolated && SharedArrayBuffer`).

**Part A (zero-copy snapshot, 2026-05-16):** `cells_snapshot_view` / `cell_inspect_view` v `crates/aenternis-wasm/src/lib.rs` vracejí `js_sys::Uint32Array::view(&buf[..])` přímo nad WASM lineární pamětí — uspoří jeden ~24 MB memcpy per snapshot na milion-cell světě. `snapshot_buf` i nový `inspect_buf` reusují kapacitu napříč ticky. Legacy `cells_snapshot` / `cell_inspect` ponechány jako safe API (kryjí host testy). Worker handler (`src/worker-handler.ts:90, 109`) přepnut na view + `new Uint32Array(view)` kopii **před** dalším WASM voláním nebo `postMessage` transferem (jinak by transfer detachl celou WASM paměť, protože view ji *aliasuje*, ne *kopíruje*).

**Divergence Part A:**

- `cell_inspect` muselo přejít z `&self` na `&mut self` kvůli sdílenému `inspect_buf` — pět host testů dostalo `let mut w` (mechanická úprava, ne sémantická).
- Workspace lint relaxován z `unsafe_code = "forbid"` na `"deny"` (`Cargo.toml:24-28`). Rust neumí přebít `forbid` z užšího scope, takže `#![allow(unsafe_code)]` lokálně v `aenternis-wasm/src/lib.rs` by jinak nešel. Sémanticky `deny` stále blokuje `unsafe` všude kromě explicitně označeného modulu; každé volání `Uint32Array::view` má SAFETY komentář s JS-side kontraktem (kopíruj před dalším WASM voláním, netransferuj buffer).

## Co se neimplementovalo

### cell-soa Etapa 2 (full SoA Cell split)

Záměrně odloženo po Etapě 1 (2026-05-12). Hypotéza zněla, že rozbití `Cell` (`cell.rs:38-77`, devět polí včetně `memory: Vec<u32>`) na separátní arrays uvnitř `Cells` zlepší cache locality a sníží bucket-size cost o ~30 % v hot loopech (`compute_natural_rates`, `refresh_neighbor_energies`). Po implementaci Etapy 1 (slot-indexed wrapper, který sám o sobě zmenšil bucket header) měření na `tick_step` ukázalo marginální win bez náznaku, že další SoA split přinese očekávaný řád — empirický signál byl ~−2.3 % na `warm_large`, v rámci šumu. Bez nového měřeného důvodu se to nedělá.

Když budoucí profil ukáže `Cell::energy()` lookup přes velký bucket jako hotspot (např. po slepé uličce s `energies` side-table, viz níže), vrátit se k tomu. Tehdy bude potřeba nový benchmark s konkrétními cíli, ne jen "očekáváme cache win".

## Co se zahodilo

### refresh-neighbor-diff (`energies` side-table)

Implementováno, otestováno, změřeno, **revertováno** stejný den (2026-05-12). Hypotéza: oddělení `energies: FxHashMap<Coord, u32>` od `cells: FxHashMap<Coord, Cell>` zmenší bucket scan v `refresh_neighbor_energies` (čte jen `Cell::energy()` přes velký bucket s `memory: Vec<u32>` heap pointerem) a zlepší cache.

Měření ukázalo opak. Na multi-core parallel path:

- `side_32` micro-bench: +14.2 % (`p = 0.00`)
- `tick_step/dense_grid/side_32`: +15.4 % (`p = 0.00`)

Side-table přidala cache contention horší než ušetřený bucket scan — paralelní zápisy na `energies` synchronizovaly cache lines napříč jádry, zatímco původní `cells` čtení byly read-only během fáze a benefitovaly z cache replikace.

Veškerý kód (pole `energies`, hooky v mutátorech, rebuild po `apply_outflow`, čtení v `refresh_neighbor_energies`, micro-bench, 14 nových testů) revertován jediným `git checkout HEAD -- crates/`. Lekce: hypotézy o cache cost musí být změřené, ne odhadnuté z bucket size — multi-core efekty mění znaménko výsledku.

Tento problém (drahé `Cell::energy()` čtení přes velký bucket v parallel refresh) subsumuje `cell-soa` Etapa 2: pokud SoA split někdy proběhne, `refresh_neighbor_energies` bude číst úzkou `energy` arenu bez side-table a bez duplikace dat. Než k tomu dojde, refresh zůstává v původní podobě.
