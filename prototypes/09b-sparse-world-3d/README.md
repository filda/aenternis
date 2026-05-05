# Prototyp 9-B: sparse svět (3D)

Přesná kopie prototypu 9 s jediným rozdílem: **`DIRS = 4 → 6`**. Sparse svět dostává třetí osu `z` se směry `zp` a `zn`, vše ostatní je stejné — žádné nové opcody, žádná nová pravidla, jen šest sousedů místo čtyř. Sparse model nemá topologii toroidu (a neměl ji ani v 9, ten název nese jen `toroid.js` jako reference implementace pro comparison harness). Otázka, kterou prototyp ověřuje, je tedy **drží se mechanika sparse světa beze změny i ve 3D, nebo se objeví edge case, který v rovině nevidíme?**

Detailní designový plán prototypu 9 je v `docs/prototype-09-plan.md`. Tento README je laboratorní zápisník 3D varianty.

## Spuštění

Otevři `index.html` v prohlížeči. Bez build kroku.

Pro headless ověření (Node):

```sh
# 3 scénáře × 200 ticek, konzervace + cap
E_TOTAL=512 TICKS=200 node test-headless.js

# Equivalence proti reference 3D toroidu (port `toroid.js` z prototypu 9 rozšířený na 6 směrů; sparse svět sám toroid není)
PROG=pure_noise          E_TOTAL=64 TICKS=200 N=32 node test-equivalence.js
PROG=counter             E_TOTAL=64 TICKS=200 N=32 node test-equivalence.js
PROG=self_xp_replicator  E_TOTAL=64 TICKS=100 N=32 node test-equivalence.js
```

V UI je tlačítko **„10 000 ticek (3 scénáře)"**, které ověří konzervaci a strop ve všech třech scénářích pro aktuálně nastavené `E_total`.

## Centrální invariant

> Buňka existuje právě tehdy, když má nenulovou energii. Počet existujících buněk je v každém okamžiku roven počtu jednotek energie, který je v nich aktuálně rozdělen.

Důsledek: `world.size() ≤ E_total`. Maximální průměr světa v jednotkách buněk je v 3D `O(∛E_total)` (oproti `O(√E_total)` v 2D), takže expanze fronty je v jednotlivých osách pomalejší. `E_total` zůstává **jediný globální parametr světa** — žádný `gridSize`, žádná pevná topologie, vše ostatní emerguje.

## Co se proti prototypu 9 změnilo

Fyzika je identická. Rozdíl je v dimenzi a tedy v počtu směrů:

- **`DIRS = 6`.** Pořadí směrů: `xp, xn, yp, yn, zp, zn`. `OPPOSITE = [1, 0, 3, 2, 5, 4]`. `LAYOUT_ORDER_FROM_END = [5, 4, 3, 2, 1, 0]` (od `zn` po `xp`).
- **3D coord packing.** `packCoord(x, y, z)` uloží tři 32-bit signed čísla do bigint klíče (bity `[64..96)` = x, `[32..64)` = y, `[0..32)` = z).
- **Per-cell seed bere z.** `cellSeed(worldSeed, x, y, z)` přidává mixing krok pro `z` se třetím prvočíslem (1274126177); jinak je hash totožný se 2D verzí.
- **Big bang v `(0, 0, 0)`.** Jedna buňka v počátku 3D mřížky drží celé `E_total`. Po prvním ticku má svět typicky 7 buněk (původní + 6 sousedů).
- **Centroid + bbox v 3D.** `boundingBox()` vrací `{ xMin..xMax, yMin..yMax, zMin..zMax }`, `centroid()` vrací `{ x, y, z }`.
- **3D toroid pro equivalence test.** `toroid.js` je `N×N×N` grid s wrap-around ve všech třech osách. Defaultní `N=32` (≈ 32 768 buněk) drží paměť a runtime na rozumné úrovni; pro 2D byl default `N=64` (4 096 buněk), v 3D by to bylo 64³ = 262 144 buněk a test by trval minuty.
- **UI: three.js viewer.** Sparse svět se renderuje přes WebGL (three.js r146 UMD ze CDN), `InstancedMesh` boxů s kapacitou rovnou aktuálnímu `E_total`. Per-frame se nastavuje `mesh.count = world.cells.size` a pro každou živou buňku `setMatrixAt + setColorAt`. OrbitControls (drag = rotace, scroll = zoom, pravé tl. = pan) plus WSAD + Q/E pro pohyb kamery (Shift = sprint). Bbox je dynamický drátový kvádr (`LineSegments` z `EdgesGeometry`), origin červený 3-osý křížek, centroid bílá koule. Defaultní kamera míří na centroid a volitelně ho i sleduje. Volitelná vizualizace: energie (heat paleta), `mem[top]`, `mem[bottom]`, origin tag (HSV podle hashe). Kvůli three.js dependency je teď UI závislé na CDN — testy v Node (`test-headless.js`, `test-equivalence.js`, `test-debug.js`) běží beze změny, protože importují jen `world.js` / `toroid.js`.

Žádné nové opcody nejsou potřeba. Asembler rozpoznává `zp` a `zn` stejně jako `xp/xn/yp/yn`. Programy z prototypu 9 napsané pro 4 směry běží v 3D světě beze změny — vidí jen 4 sousedy z 6 a `senergy` na nepoužité ose stále vrací 0 pro neexistujícího souseda, což je stejné chování jako v 2D sparse.

## Pořadí událostí v ticku

Beze změny proti prototypu 9:

1. CPU fáze
2. Sub-tick reflow s combined_rate
3. Outflow snapshot
4. Inflow + alokace (target buňka, pokud neexistuje, je alokována s prázdnou pamětí)
5. Reset active outflow + override flagů
6. GC: smaž buňky s E = 0
7. Layout pro další tick + refill budgetů

Iterace přes `DIRS` se z 4 na 6 změní v každé fázi, kde se sčítá nebo prochází per-směr stav (rates, activeOutflow, pointers, pointerOverridden).

## Ověření

Implementace prošla těmito testy (viz `test-headless.js` a `test-equivalence.js`):

**Konzervace energie do bitu.** `sum(cell.energy) == E_total` v každém ticku. Ověřeno pro 3 scénáře (`pure_noise`, `counter`, `self_xp_replicator`) na `E_total = 64` i `512` přes 200 ticek; UI batch test umí 10 000.

**Strop velikosti.** `world.size() ≤ E_total` ve všech ticích. V 3D dosahuje `pure_noise` při `E_total = 512` peak ~500 buněk podobně jako v 2D — heat-death-like režim.

**Bitová ekvivalence se 3D toroidem.** Pro `pure_noise` a `counter` souhlasí 256 ticek bit-identicky proti 3D toroidu `32×32×32` (na ticku 257 sparse front přeleze toroid bbox a test se ukončí — předchozí ticky souhlasily); na `N=40` projde 300 ticek za ~1 minutu. Pro `self_xp_replicator` 100 ticek na `N=32`. Test platí, dokud sparse svět zůstává uvnitř toroidního bbox `[-N/2 .. N/2 - 1]` na všech třech osách. V 3D je toroid `O(N³)` paměť per tick, takže velký `N` jako v 2D verzi (`N=128`) není praktický.

## Pozorování

**Big bang v 3D je stále organický.** Po prvním ticku je svět typicky 7 buněk (původní + 6 sousedů), oproti 5 v 2D. Tvar fronty po desítkách ticek je v `pure_noise` přibližně sférický, nikoli diskový — což je očekávané, protože difuze je izotropní v počtu sousedů.

**Rychlost expanze fronty zůstává ≈ 1 buňka / tick / strana**, protože pravděpodobnost vzniku nového souseda je pořád `coeff` per směr a směrů je víc. Ve scénářích bez programu nebo se slabým programem se objem světa škáluje jako `O(t³)` namísto `O(t²)`, takže heat-death dorazí pro stejné `E_total` o tolik dříve.

**Heat death v 3D nastává rychleji.** Při `E_total = 512` v 2D dorazí `world.size() ≈ E_total` po stovkách ticek (front je 2D); v 3D, kde se prostor zaplňuje rychleji, by `pure_noise` mohl při dostatečném počtu ticek dorazit ke stropu dřív (viz batch test).

**Self-replicator drží svět kompaktnější** stejným mechanismem jako v 2D — opakovaný reseed memory v `xp` směru udržuje koncentraci energie bez ohledu na počet os.

## Vyhodnocení

**Bylo to laciné?** Ano. Sparse 3D je doslova mechanická obměna 2D verze: `DIRS = 4 → 6`, coord pack 64-bit → 96-bit (přes bigint), `[dx, dy] → [dx, dy, dz]`. Žádný nový edge case se neobjevil. Comparison harness proti 3D toroidu prošel bez zvláštních úprav, jen s menším defaultním `N` kvůli paměťovým nárokům `N³` gridu.

**Komplikuje to programování?** Ne. Programy napsané pro 4 směry běží beze změny; přidávají se jen `zp/zn` jako další směry, na které lze cílit `setp`, `port`, `senergy`. Žádné nové opcody.

**Jaký je perf strop?** UI se 2D canvas rendererem zvládá `E_total = 512` a typických ~500 buněk při ~30 fps. Headless reference (3D toroid v `toroid.js`) `32³` × 1000 ticek je výrazně pomalejší než 2D `64²` (32 768 vs. 4 096 buněk × větší per-tick cost), ale stále v řádu desítek sekund. Pro vyšší škály patří 3D do produkčního Rust + WASM kódu (viz `docs/plan.md`).

**Rozhodnutí.** Tento prototyp existuje pro kompletnost — odpovídá na otázku „co když mechaniku zkusíme přepnout na 3D, vznikne něco?". Odpověď: nevznikne, vše prochází. Posiluje to pozici, že se sparse fyzika dá portovat rovnou do produkčního Rust kódu, kde 3D je default a 2D je jen speciální případ s `Nz = 1`.

## Známá omezení

Stejná jako u prototypu 9, plus:

- **Žádný inspector buněk.** Klik na voxel zatím nic nedělá (raycast picking jen TODO).
- **Žádný entity tracking.**
- **Žádná stopa historicky obsazených pozic.**
- **`MAX_MEMORY = 65536` cap.** Stejný důvod jako v 2D.
- **Voxel mesh kapacita = `E_total`.** Měnit `E_total` za běhu znamená realokovat InstancedMesh; primární cesta je proto Reset. Safety check v render loop přealokuje, kdyby se `world.size()` přesto dostala přes kapacitu, ale to by indikovalo bug v cap invariantu.
- **CDN dependency.** UI vyžaduje přístup k `unpkg.com` pro three.js a OrbitControls; testy v Node tuto závislost nemají.
- **Reference grid `N=32`.** Comparison harness (`toroid.js`, `N×N×N` mřížka s wrap-around — to je jediné místo, kde má smysl mluvit o toroidu) má memory footprint `N³` × cell, který roste rychle. Pro `N=64` je cca 8× pomalejší a 8× větší paměť — pro 200 ticek `self_xp_replicator` zvládá `N=32`, pro pomalu se šířící programy (`counter`) je `N=32` dostatečný i na 1000 ticek.

## Soubory

- `world.js` — `SparseWorld` třída (3D), asembler, exporty (browser + Node)
- `toroid.js` — port 3D toroidu s per-tick rng pro fair ekvivalenci (jen pro test)
- `test-headless.js` — Node test konzervace + cap
- `test-equivalence.js` — Node test bit-identity proti 3D toroidu
- `test-debug.js` — diagnostický dump pro hledání divergencí
- `index.html`, `main.js`, `styles.css` — UI s izometrickým 2D rendererem
- `package.json` — `"type": "commonjs"` override (root projektu má `"type": "module"`)
