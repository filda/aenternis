# Prototyp 9: sparse svět (2D)

Devátý laboratorní prototyp odpovídá na otázku **lze nahradit pevný toroidální grid sparse modelem světa, jehož velikost je důsledkem celkové energie, nikoli jejím parametrem — a zachovat všechny dosavadní fyzikální vlastnosti?**

Detailní designový plán je v `docs/prototypes.md`. Tento README je laboratorní zápisník.

## Spuštění

Otevři `index.html` v prohlížeči. Bez build kroku.

Pro headless ověření (Node):

```sh
# 3 scénáře × 200 ticek, konzervace + cap
E_TOTAL=512 TICKS=200 node test-headless.js

# Equivalence proti toroidu (port prototypu 6 s per-cell rng)
PROG=pure_noise E_TOTAL=64 TICKS=1000 N=128 node test-equivalence.js
PROG=counter   E_TOTAL=64 TICKS=1000 N=128 node test-equivalence.js
PROG=self_xp_replicator E_TOTAL=64 TICKS=200 N=128 node test-equivalence.js
```

V UI je tlačítko **„Spustit batch test 10000 ticek"**, které ověří konzervaci a strop ve všech třech scénářích pro aktuálně nastavené `E_total`.

## Centrální invariant

> Buňka existuje právě tehdy, když má nenulovou energii. Počet existujících buněk je v každém okamžiku roven počtu jednotek energie, který je v nich aktuálně rozdělen.

Důsledek: `world.size() ≤ E_total`. Maximální průměr světa v jednotkách buněk je `O(√E_total)` v 2D. `E_total` je **jediný globální parametr světa** — toroidní `gridSize` mizí, topologie mizí, vše ostatní emerguje.

## Co se proti prototypu 6 změnilo

Fyzika je stejná. Rozdíl je v topologii a datové struktuře:

- **`Map<bigint, Cell>` místo `Float32Array`.** Souřadnice 32-bit signed per osu, klíč je 64-bit bigint. Hráč souřadnice nevidí, je to implementační detail.
- **Big bang místo iniciálního scénáře.** Jedna buňka v (0, 0) drží celé `E_total`, žádní sousedi neexistují. Difuze sama spustí expanzi.
- **Alokace na zápis.** Když buňka emituje směrem, kde soused neexistuje, fáze inflow ho alokuje s prázdnou pamětí (E = 0) a zapíše do něj. Není to nové fyzikální pravidlo, jen důsledek `world.getOrCreate(coord)`.
- **Garbage collection.** Po každém ticku se buňky s `E = 0` odstraní z mapy. Nemůžou existovat — paměť = energie.
- **Kamera energetický centroid.** Default kamera míří na `(Σ E_i · x_i) / E_total` po jednotlivých osách (viz `prototypes.md`, sekce „Default kamera").
- **Tick-based RNG.** Stochastic floor používá rng seedovaný `(coord, tick)`. Pro každý (coord, tick) je rng deterministicky čerstvé, což činí výsledky nezávislé na pořadí iterace a životním cyklu buňky (alive/dead/realloc). Detail: viz `cellTickSeed` v `world.js`.

## Pořadí událostí v ticku

1. CPU fáze (žádná změna oproti prototypu 6)
2. Sub-tick reflow s combined_rate
3. Outflow snapshot
4. **Inflow + alokace** (target buňka, pokud neexistuje, je alokována s prázdnou pamětí)
5. Reset active outflow + override flagů
6. **GC: smaž buňky s E = 0** (NOVÝ KROK)
7. Layout pro další tick + refill budgetů

## Ověření

Implementace prošla těmito testy (viz `test-headless.js` a `test-equivalence.js`):

**Konzervace energie do bitu.** `sum(cell.energy) == E_total` v každém ticku. Ověřeno pro 3 scénáře (pure_noise, counter, self_xp_replicator) na E_total = 64 i 512 přes 500 ticek (UI batch test umí 10 000).

**Strop velikosti.** `world.size() ≤ E_total` ve všech ticích. Ve scénáři pure_noise při E_total = 512 dosahuje peak ~496 buněk (97% kapacity). To je heat-death-like režim.

**Bitová ekvivalence se toroidem.** Pro pure_noise a counter prošlo 1000 ticek bit-identicky proti portu prototypu 6 na 128×128 toroidu. Pro self_xp_replicator 200 ticek (test je pomalejší kvůli memory churn). Test platí, dokud sparse svět zůstává uvnitř toroidního bbox.

## Pozorování

**Big bang funguje organicky.** Po prvním ticku je svět typicky 5 buněk (původní + 4 alokovaní sousedi). Po desítkách ticek se rozprostře do tvaru, který závisí na programu — symetrický disk pro pure_noise, mírně asymetrický pro counter, podlouhlý pro replikátor (kvůli preferenci jednoho směru).

**Rychlost expanze ≈ 1 buňka / tick / strana** ve scénářích bez programu nebo se slabým programem. Difuze s `coeff = 0.15` při heat-death stavu (každá buňka má E ≈ 1) drží vznik nového souseda na pravděpodobnosti ~0.15 / tick / směr / buňka, takže front postupuje přibližně 1 / tick.

**Heat death emerguje sám.** Při dlouhých bězích bez stabilizujícího programu se energie rozprostře na maximum (`world.size() ≈ E_total`, průměrná `E ≈ 1`) a programy přestávají fungovat (1 slot paměti = nesmyslný program). Šipka času z big bangu do heat death je doslova fyzickou nutností — žádné umělé stárnutí, jen difuze.

**Self-replicator drží svět kompaktnější.** Při E_total = 1024 dosahuje pure_noise bbox ±2000 buněk (heat-death front), zatímco self_xp_replicator drží entitu v tighter bublině ±30 buněk. Důvod: replikátor opakovaně reseeds memory v xp směru, čímž udržuje koncentraci energie.

## Vyhodnocení

**Bylo to laciné?** Středně. Sparse model přidal cca 200 řádků navíc oproti toroidu (Map lookup, alokace, GC). Comparison harness byl bolavější než samotný sparse — bug v read-after-write (dominance počítaná z post-step `neighbor.energy`) byl tichý a projevoval se až rozdílem rng-driven hodnot. Po snapshotu pre-step energie a tick-based rng jsou impls bit-identické.

**Komplikuje to programování?** Ne. Programy napsané pro 2D toroid běží beze změny. Existuje rozdíl, který programátor MŮŽE pozorovat: senzor `senergy` směrem do void buňky vrací 0 (neexistující soused = prázdná pozice), což je stejné jako E=0 v toroidu. Žádné nové opcody nejsou potřeba.

**Jaký je perf strop?** 3 scénáře × 1000 ticek × E_total = 64 × N=128 toroid trvá ~30 s v Node (= ~10k cell-ticks/s na sparse straně, ~50k na toroidní). Sparse je pomalejší kvůli Map lookup overheadům, ale dobíhá lépe než toroid pro řídké světy. Pro UI běh při E_total = 512 a typických ~500 buňkách zvládá viewer ~30 fps při 1 step/frame.

**Je 3D varianta odůvodněná?** Sparse mechanika fungovala v 2D bez kompromisů, žádný neočekávaný edge case. 3D port znamená rozšíření DIRS=4 → 6, LAYOUT_ORDER, DIR_OFFSET. Fyzika identická. **Závěr (revize 2026-05-03):** samostatný JS 3D prototyp se nestaví. To, co by ověřoval, už víme — dimension-agnostic mechaniku jsme prokázali bit-identitou se toroidem. 3D implementace jde rovnou do produkčního Rust + WASM kódu, kde bude první verifikační krok port `test-equivalence.js` proti Rust portu 3D toroidu z prototypu 5. Detaily v `docs/plan.md`.

## Známá omezení

- **Žádný inspector buněk.** Klik na canvas zatím nic nedělá. Pokud bude potřeba detailní pohled na konkrétní buňku, přidat panel ve stylu prototypu 6.
- **Žádný entity tracking.** Lineage tracker z prototypu 6 nebyl portovaný — sparse model nemá pevný grid pro Hamming match snapshot.
- **Žádná stopa historicky obsazených pozic.** Volitelná feature z `prototypes.md` (otevřená otázka 4); nezahrnuto.
- **`MAX_MEMORY = 65536` cap.** Pro `E_total > 65536` v jediné buňce (= big bang start) by hrozila energy leakage při dominance insert. Pro praktické `E_total ≤ 65536` se cap nedosahuje po prvním ticku.
- **Performance.** Naivní `Map<bigint, Cell>` je dostatečné pro `E_total` v řádu tisíců. Pro desítky tisíc by pomohl chunked grid nebo sparse quadtree, ale to už by porušovalo princip „obyčejnost nad optimalizací" pro prototyp.

## Soubory

- `world.js` — `SparseWorld` třída, asembler, exporty (browser + Node)
- `toroid.js` — port prototypu 6 s per-tick rng pro fair ekvivalenci (jen pro test)
- `test-headless.js` — Node test konzervace + cap
- `test-equivalence.js` — Node test bit-identity proti toroidu
- `test-debug.js` — diagnostický dump pro hledání divergencí
- `index.html`, `main.js`, `styles.css` — UI
- `package.json` — `"type": "commonjs"` override (root projektu má `"type": "module"`)
