# Aenternis — plán prototypu 9: sparse svět (2D)

Last updated: 2026-05-03 (2D-first scoping)

Tento dokument je detailním plánem laboratorního prototypu `09-sparse-world`. Designový kontext žije v `aenternis.md` a `mechanics.md`; obecný roadmap je v `plan.md`. Po dokončení prototypu se sem doplní záznam výsledků a dokument zůstane jako historický záznam návrhu (analogicky k tomu, jak fungovaly plány předchozích prototypů před konsolidací).

**Dimenzionalita:** 2D. Sparse mechanika je dimension-agnostic, ale 2D je pro odladění výrazně levnější (4 směry místo 6, viewer je triviální canvas mřída, bugy jsou okamžitě vizuálně vidět). Toto rozdělení kopíruje existující vzor projektu (prototyp 5 = 3D, prototyp 6 = 2D). 3D varianta **se po dokončení tohoto prototypu nestává samostatným JS prototypem** — viz sekce „Návaznost na ostatní práci" na konci dokumentu.

## Otázka, kterou prototyp odpovídá

Lze nahradit pevný toroidální grid **sparse modelem světa, jehož velikost je důsledkem celkové energie, nikoli jejím parametrem** — a zachovat při tom všechny dosavadní fyzikální vlastnosti (difuze, gradient, sub-tick reflow, dominance) bez výjimek?

Konkrétně: chování systému musí být v překrytí dvou modelů (toroidní grid versus sparse svět) numericky shodné, dokud expanze nedosáhne hranice toroidu. Liší se až ve chvíli, kdy by toroid musel obtočit svět — zatímco sparse svět prostě dál neexistuje.

## Centrální invariant

> **Buňka existuje právě tehdy, když má nenulovou energii. Počet existujících buněk světa je tedy v každém okamžiku roven počtu jednotek energie, který je v nich aktuálně rozdělen.**

Důsledek: maximální počet buněk = `E_total`. Maximální průměr světa v jednotkách buněk je stropově `O(√E_total)` v 2D variantě (kdyby se energie rozprostřela do kompaktního disku s hustotou 1) a `O(∛E_total)` v budoucí 3D variantě. Reálná konfigurace má menší poloměr a delší tvar (chapadla, nehomogenita).

`E_total` je tím **jediný globální parametr světa**. Mizí parametr `gridSize`. Mizí topologie (toroid). Vše ostatní emerguje z dynamiky difuze a programu.

## Big bang jako počáteční podmínka

Svět startuje v konfiguraci `1×1`: jedna buňka na souřadnici `(0,0)`, držící celou energii `E_total`. Žádní sousedi neexistují. Program této buňky se generuje z deterministického seedu (xorshift / PCG, viz `questions.md`).

V prvním ticku buňka:

1. Zjistí natural_rate pro všechny 4 směry. Protože všichni sousedi mají `E_neighbor = 0` (neexistují, takže se chovají jako prázdné), `natural_rate[d] = stochasticFloor(E_self * coeff)` ve všech směrech. To je výrazný outflow ve všech 4 směrech najednou.
2. Volitelně provede CPU fázi (jestli má program v low-address kódu).
3. Outflow fáze: každý směr alokuje nového souseda v adekvátní pozici a zapíše do něj odpovídající slice paměti.

Tj. po prvním ticku má svět typicky 5 buněk (původní + 4 nově alokovaných sousedů), s energií rozdělenou podle gradientů. Symetrie je perfektní jen pokud je program perfektně symetrický — což být nemusí, protože pointery nedrží symetrii v pořadí (xp, xn, yp, yn).

## Co se nemění

Většina fyziky je definovaná čistě lokálně přes face mezi buňkou a sousedem, takže přechod ze toroidu do sparse světa ji nechává nedotčenou. Konkrétně se přejímá VM a fyzikální parametry z **prototypu 6** (2D toroid, 20 opcodů):

- VM, opcode set (20 opcodů), slot model, K = 1
- Pointer layout od konce paměti pro 4 směry, sub-tick reflow s combined_rate, override pointerů přes setp/setpv
- Stochastic floor flow, proportional clamping, K-based CPU rate
- Difuzní koeficient v rozsahu 0.15-0.30 stejně jako v prototypu 6 (viz `mechanics.md`)
- Princip jádro/membrána (emerguje z dynamiky, není pravidlem)
- Identity je interpretace, ne stav
- Diagonální pohyb je dál fyzicky nemožný
- Dominance / intrusion mechanika (až bude implementovaná v prototypu 6)

Programy napsané pro toroidní variantu prototypu 6 by měly v sparse světě fungovat **identicky**, dokud nenarazí na situaci, která v toroidu nemůže nastat (sousední pozice neexistuje, tj. dokud se sparse svět nenapne přes hranice toroidu).

## Co se mění

### 1. Datová struktura světa

Z `Float32Array` indexovaného `idx = y*N + x` (toroid 2D z prototypu 6) se přechází na `Map<packedCoord, Cell>`, kde `packedCoord` je 64-bit celočíselné zabalení `(x, y)` — typicky `(BigInt(x) << 32n) | BigInt(y >>> 0)` nebo ekvivalent. Souřadnice jsou 32-bit signed (záporné jsou potřeba — svět expanduje všemi směry od počátku).

Cell sám obsahuje totéž co dosud (energy, memory `Uint32Array`, PC, 4 pointery, override flags) — interní reprezentace buňky se nemění.

### 2. Neighbor lookup

```
getNeighbor(x, y, dir) -> Cell | undefined
```

Místo modulárního indexu hash lookup. Když vrátí `undefined`, soused fyzicky neexistuje. V difuzní fázi to znamená `E_neighbor = 0` (gradient počítán normálně). V outflow fázi to znamená trigger pro alokaci.

### 3. Iterace aktivních buněk

`world.forEachCell(callback)` se mění z lineárního průchodu polem na `map.values()`. Iteruje se pouze to, co existuje — což je horní mez `E_total`, takže je to v praxi méně než iterace celého toroidu pro stejné `E_total`.

Důsledek pro vykreslování: viewer musí umět dynamický bounding box a prázdné buňky (v 2D canvas mřížce stačí "nevykresli pokud not in map").

### 4. Alokace souseda při outflow do void

Pokud při outflow buňka emituje směrem, kde soused neexistuje, fáze inflow vytvoří nového souseda jako standardní side-effect zápisu. Pravidlo:

```
if (!world.has(targetCoord)) {
    world.allocate(targetCoord);   // vznikne buňka s prázdnou pamětí, E = 0
}
target.appendInflow(slots, direction);
```

Buňka se tedy nealokuje s "iniciálním programem" — alokuje se prázdná a obsah jí dodá první inflow. To je čistě konzistentní s principem "obsah paměti je to, co do ní bylo zapsáno". Žádné default values, žádný iniciální stav, který by porušoval fyziku.

### 5. Dealokace buňky s nulovou energií

Pokud po outflow + inflow fázi má buňka `E = 0`, **přestává existovat** — odstraní se z mapy. Nevolá se žádný explicit destructor; kvůli `energy = memory` má taková buňka stejně 0 slotů paměti, takže se neztrácí žádná informace. Konzervační invariant zůstává: total_energy = sum(cells.energy) = počet buněk × průměrná energie.

Pozor: tohle se musí dělat **až po inflow fázi**, ne mezi outflow a inflow. Buňka, která v outflow vyhodila všechnu svou energii, ale v inflow něco přijme od souseda, nezaniká. Pořadí zánikové kontroly je za bodem 4 v cyklu ticku popsaném v `mechanics.md`.

### 6. Konkurenční alokace ve stejném ticku

Není to nové pravidlo — je to jen důsledek implementace. Inflow fáze volá `world.getOrCreate(coord).appendInflow(...)`. Rozdíl mezi "buňka už existovala" a "právě byla vytvořena" je čistě v tom, jestli `getOrCreate` udělal lookup, nebo lookup + alokaci. Mergovací logika (pořadí směrů xp, xn, yp, yn) je identická s toroidní variantou. Žádný speciální merge, žádný race, žádné nové fyzikální pravidlo.

## Pořadí událostí v ticku

Rozšíření existujícího cyklu z `mechanics.md`:

1. **CPU fáze** — žádná změna.
2. **Sub-tick reflow** — žádná změna.
3. **Outflow** — pro každý směr s nenulovým combined_rate: zkopíruj sloty z source, redukuj source paměť/energii. Přidej cílovou pozici do bufferu pending_inflows.
4. **Inflow + alokace** — pro každý záznam v pending_inflows: pokud cílová buňka neexistuje, alokuj ji (E = 0). Append slotů.
5. **Reset active outflow + override flags** — žádná změna.
6. **Garbage collection** — odstraň všechny buňky s E = 0. Toto je nový krok.
7. **Layout pro další tick** — žádná změna.

## Co je v rozsahu prototypu

Konkrétní výstup je samostatná HTML stránka v `prototypes/09-sparse-world/`, která ověří:

1. **Numerická ekvivalence s toroidem v překrytí.** Stejný počáteční stav, stejný seed, stejný program → identický průběh proti **prototypu 6 (2D toroid, 20 opcodů)**, dokud se světy nezačnou lišit (= dokud sparse svět nedosáhne hranice toroidu, nebo dokud toroidní svět nezačne potřebovat wrap-around).

2. **Konzervace energie do bitu.** `sum(cell.energy for cell in world) == E_total` v každém ticku, pro libovolný program a libovolnou délku simulace.

3. **Strop velikosti světa.** `world.size() <= E_total` ve všech ticích.

4. **Tvar expanze v praxi.** Vizualizace bounding boxu a aktivního regionu pro několik typických počátečních konfigurací: čistý šum, "god program" s těsnou smyčkou, replikátor portovaný z prototypu 6.

5. **Heat-death scenario.** Při dostatečně dlouhé simulaci a programech, které nedrží paměť pohromadě, sparse svět konverguje k maximálně rozprostřenému stavu (≈ `E_total` buněk s `E ≈ 1`). V tom stavu programy přestanou fungovat (1 slot paměti = nesmyslný program). Ověřit, že to nastává deterministicky a měřitelně.

6. **Performance v praxi.** Kolik ticek za sekundu sparse svět zvládá pro `E_total` v řádu tisíců až desítek tisíc, a jak je to v porovnání s 2D toroidem na stejné celkové energii.

## Co je explicitně mimo rozsah

- **3D varianta.** Sparse mechanika je dimension-agnostic; 3D port se po dokončení tohoto prototypu **neimplementuje jako další JS prototyp**, ale stěhuje se rovnou do produkční Rust + WASM implementace. Detail v sekci „Návaznost na ostatní práci".
- **Plnohodnotný viewer.** Stačí 2D canvas mřížka s buňkami obarvenými podle energie a textový HUD s počtem buněk + bounding boxem + centroidem. Žádné fancy přechody, žádný entity-tracking následník, žádný overlay typu "stopa migrace". Cílem viewer není hra, je to debug nástroj.
- **Migrace prototypů 1-8.** Sparse svět ověřujeme v izolaci. Existující prototypy zůstávají toroidní, dokud sparse model neprojde verifikací.
- **Production refactor src/.** Sparse svět je laboratorní experiment, ne nový základ. Až po jeho ověření (a po případné 3D variantě) se rozhodne, jestli a kdy do `src/` přejít.
- **Optimalizace hash mapy.** Naivní `Map<bigint, Cell>` stačí; teprve pokud profiling ukáže, že je to bottleneck pro sledovaný rozsah `E_total`, řešit chunked grid nebo sparse quadtree.
- **Dynamická redistribuce souřadnic.** Souřadnice se nikdy nepřepočítávají, neresetují, neposouvají. Svět může mít těžiště kdekoli a kamera ho musí umět najít, ale fyzika souřadnice nikdy nemění.

## Rozhodnutí (revize 2026-05-03)

Tato sekce zachycuje rozhodnutí, která padla v debatě nad první verzí plánu.

**Souřadnicový rozsah.** Implementační detail. Hráč souřadnice nevidí. Použije se 32-bit signed integer per osu — pro `E_total` v realistickém rozsahu (do ~10⁷) je rezerva několik řádů. V prototypu se overflow neošetřuje žádnou asercí. Až v budoucnu vznikne save/load formát světa, rozsah souřadnic se v něm ukotví explicitně, ale to je mimo rozsah prototypu 9.

**Default kamera při vykreslování.** Kamera ve výchozím stavu míří na **těžiště vážené energií** — energetický centroid `(Σ E_i · x_i) / E_total` po jednotlivých osách. Bounding box střed by skákal vždy, když se na okraji objeví nebo zmizí slabá buňka, zatímco energetický centroid se hýbe hladce a intuitivně sleduje "kde se něco zásadního děje". Když hráč klikne na konkrétní buňku, kamera přepne do entity-tracking módu (analog k existujícímu sledování buňky s nejvyšší energií v prototypu 8). Plynulý přechod mezi módy je nice-to-have, ne blokr.

**Konkurenční alokace.** Není nové pravidlo, viz sekce "Co se mění" → bod 6.

## Otevřené otázky pro `questions.md`

Tyto body se přidají do `questions.md` souběžně s tímto plánem:

1. **Pohlcení existujícího "Movement only into an empty cell" záznamu.** Tato historická otázka byla rozhodnuta v rámci toroidu (prázdné buňky neexistují, pohyb řeší dominance). V sparse světě má sekundární význam: prázdná buňka skutečně neexistuje (není alokovaná). Pohyb do "neexistujícího souseda" znamená alokaci nového souseda = standardní side-effect emise. Tj. existing rule o tom, že pohyb řeší dominance, platí dál — protože alokace se tváří přesně jako emise do existující buňky s `E = 0`.

2. **Pořadí buněk pro deterministickou iteraci.** `Map.values()` v JavaScriptu zachovává insertion order. Pro deterministické experimenty (stejný seed → stejný průběh) je to dostatečné, ale závisí to na tom, v jakém pořadí byly buňky alokovány — což závisí na pořadí směrů emise. Toto pořadí musí být explicitní (stejné napříč ticky a napříč prototypy), aby reprodukovatelnost držela.

3. **Energie v jediné buňce na začátku.** `E_total` může být potenciálně velké (desítky tisíc). Iniciální buňka má `E_total` slotů, což je hodně paměti pro jednu buňku. CPU fáze v prvním ticku by chtěla `floor(E_total / K)` instrukcí, což pro `K = 1` znamená desítky tisíc instrukcí v prvním ticku, ještě než cokoli odteče. To je v pohodě teoreticky, ale prakticky chce ošetřit (rate-limit první tick? nebo nechat jak je a smířit se s tím, že big bang je pomalý tick?).

4. **Trace historicky obsazených pozic v UI.** Když svět vyrostl do oblasti A, pak entita migrovala do oblasti B a A se "vyprázdnila" (všechny buňky tam mají `E = 0` → byly dealokovány), je v UI rozdíl mezi "tam svět nikdy nebyl" a "tam svět byl, ale už není"? Pro fyziku irrelevantní, pro debugging ("kudy entita migrovala?") možná užitečné. Volitelná feature, ne blokr implementace fyziky.

5. **Vztah ke konsolidovanému rozhodnutí "Energy conservation strict: world_total = N³ always".** V toroidu byla rovnost `world_total = N³` výchozím stavem (každá buňka start s E = 1). V sparse světě je `world_total` zadáno explicitně jako `E_total` — bez vazby na nějaké N. Konsolidované rozhodnutí se tím nezpochybňuje, jen se přepisuje pojem: konzervace platí absolutně, hodnota se zadává jiným kanálem.

## Návaznost na ostatní práci

Sparse svět **nepředchází** dominance / intrusion mechaniku. Naopak: dominance se má nejdřív implementovat v prototypu 6 (toroid 2D), aby fyzika kolize byla odladěná na pevném gridu, a teprve pak se sparse model staví na hotovém modelu fyziky. Když sparse svět projde, dominance v něm "jen funguje" beze změny pravidel.

Sparse svět **nahrazuje** to, co bylo v `plan.md` zatím rezervované jako "Prototype 9 (self-encapsulation)". Self-encapsulation se posune o jedno číslo dál (prototype 10) — vlastně do něj sparse svět zapadá lépe než do toroidu, protože "obklopit se vlastními sousedy" je v sparse světě fyzicky doslova "vyrobit si svůj vlastní okolní svět z vlastní energie".

3D varianta **se neplánuje jako samostatný JS prototyp** (rozhodnuto 2026-05-03 po dokončení tohoto prototypu). Prototyp 9 ukázal, že sparse mechanika je dimension-agnostic v silném smyslu: `DIRS = 4 → 6`, `DIR_OFFSET` se rozšíří, `LAYOUT_ORDER_FROM_END` přidá dva směry, a tím to končí. Žádný emergentní jev specifický pro 3D, který by se v 2D neprojevil, neexistuje. Stavět JS 3D prototyp jen pro „ověření, co už víme" by bylo busy-work proti duchu prototypové fáze.

Místo toho se 3D port stěhuje **rovnou do produkční Rust + WASM implementace**. Tam je 3D od první iterace cílový stav. První produkční milestone je port `test-equivalence.js` do Rust jako bit-identity harness proti Rust portu 3D toroidu z prototypu 5 — stejná verifikační brána, kterou prošla JS verze, ale rovnou v produkčním kódu.

## Definice hotového

Prototyp je hotov, když:

- HTML stránka v `prototypes/09-sparse-world/` se otevře a běží v prohlížeči
- Konzervační kontrola (sum E == E_total) projde po 10 000 ticích pro tři různé počáteční programy
- Strop `world.size() <= E_total` projde po 10 000 ticích
- Numerická ekvivalence proti **prototypu 6 (2D toroid)** na stejném počátečním stavu (který se vejde do gridu prototypu 6) projde po 1 000 ticích
- Heat-death scenario je pozorovaný a zdokumentovaný v README prototypu (kdy nastane, jaký je terminální tvar, jaký je čas do něj)
- 2D canvas viewer ukazuje aktuální stav světa s buňkami obarvenými podle energie a HUD s počtem buněk + bounding boxem + centroidem
- README prototypu zhodnocuje: bylo to laciné? komplikuje to programování? jaký je perf strop? je 3D varianta odůvodněná?
- `prototypes.md` má nový záznam o prototypu 9 a `plan.md` ho přesouvá z plánovaného do hotového

## Výsledky implementace (2026-05-03)

Všechny body definice hotového jsou splněné, viz `prototypes/09-sparse-world/README.md` pro detailní laboratorní zápis. Stručně:

- Konzervace bit-perfect, cap drží — ověřeno v `test-headless.js` (3 scénáře × 500 ticek headless v Node, 10 000 ticek v UI tlačítku „Spustit batch test").
- Equivalence se toroidem bit-identická na 1000+ ticek pro pure_noise a counter, 200+ ticek pro self_xp_replicator (test je pomalejší, ne divergentní).
- Při řešení equivalence vyplul jeden netriviální bug, který stojí za zaznamenání pro budoucí implementace: dominance computation čte `neighbor.energy`, které mezi outflow a inflow fází může být přepsáno (záleží na pořadí iterace cílů). Oprava: snapshot pre-step energií před fází 4. Plus přechod na tick-based RNG (`cellTickSeed(seed, x, y, tick)`), aby výsledek nezávisel na životním cyklu buňky (alive/dead/realloc). Obě úpravy jsou v `world.js` a v port-of-toroid `toroid.js`.
- 3D varianta: po dokončení prototypu (revize 2026-05-03) jsme rozhodli **přeskočit JS 3D prototyp** a 3D implementovat rovnou v produkčním Rust + WASM kódu. Důvod: sparse mechanika prokazatelně dimension-agnostic, žádný edge case specifický pro 3D, který by se v 2D neprojevil. JS 3D prototyp by byl scaffolding pro něco, co už víme. Detail v `plan.md` a v sekci „Návaznost na ostatní práci".
