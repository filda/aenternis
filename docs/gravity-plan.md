# Aenternis — plán: gravitace, tlak a hustotní mutace

Last updated: 2026-06-13 (✅ IMPLEMENTOVÁNO — viz „Stav implementace" na konci)

Designový kontext žije v `aenternis.md` a `mechanics.md`; obecný roadmap v `plan.md`;
laboratorní ověření fyziky v `prototypes/11-gravity/`. Tento dokument shrnuje
rozhodnutí z návrhové diskuse a popisuje, jak gravitaci/tlak a hustotně vázanou
mutaci zasadit do produkčního jádra (`crates/aenternis-core/src/tick.rs`) se
zachováním determinismu a konzervace energie.

## Otázka, kterou plán odpovídá

Vyzařování přesouvá energii **ven** a je směrově různé (anizotropní gradient).
Gravitace by působila **dovnitř**, izotropně, a jen na část energie, která se
pro tyto účely chová jako **hmota**. Co to udělá s dynamikou světa a jak to
zapadne do stávajícího modelu toku (natural rate → outflow → inflow), aniž by
se rozbil determinismus nebo konzervace?

## Koncept

- **Hmota = zlomek energie** (`m = α·E`, např. α ≈ 0,05). Hustota je tím úměrná
  energii (buňka = jednotkový objem). `α` není schovaná rezerva — je to
  **vazebná konstanta** gravitace vůči radiaci (počítá se z aktuální `E` každý
  tick, nemá vlastní paměť).
- **Gravitace** táhne energii k hmotě, **dlouhodosahově se slábnutím**
  (potenciálové jádro `1/r` ⇒ síla `~1/r²`). Je to koncentrační síla soupeřící
  s radiací; umí téct i proti gradientu energie (akrece).
- **Tlak** je protitlak rostoucí s hustotou (`Π(E) = pressure · eref · (E/eref)^γ`,
  γ > 1). Strmý: při běžné hustotě mírný, při extrémní prudce přetlačí gravitaci
  a zabrání singularitě. **Kde se protnou křivky gravitace a tlaku, určuje
  charakteristickou hustotu/velikost vzniklých struktur.**

Bilance: **radiace + tlak = ven, gravitace = dovnitř.** Když v buňce gravitace
přetlačí, nastane čistý přítok → bohatne kdo má → kolaps; tlak ho nahoře
zastaví. To je ingredient pro vznik struktur, který čistě difuzní (vyhlazující)
model nemá.

## Co ověřil prototyp 11

Samostatná JS reimplementace (hustá 3D mřížka, float), tři síly jako tok přes
stěny. Výsledky (200 taktů, 32³, ověřeno headless):

- **Konzervace přesná** (`sum + leaked = E_total` na nulu), žádné NaN.
- **Void hranice je hlavní úskalí.** S voidem energie radiuje ke kraji a utíká →
  ze šumu mizí energie a třesk zůstane jako jedna chladnoucí koule. **Na toru se
  třesk sám roztříští** (~3400 shluků i s výchozími silami, 0 % ztrát). V
  produkčním (sparse/void) světě tedy struktury vznikají hůř — viz „Otevřené
  otázky".
- **Škála shluků se ladí silou gravitace** vůči radiaci: slabá gravitace → mnoho
  mírných shluků; silná gravitace / slabá radiace → méně, ale hustších a
  hmotnějších.

### Klíčové rozhodnutí: žádná setrvačnost

Model je **prvního řádu / přetlumený** — jediný stav je hustota, tok ∝ gradient.
Nemá hybnost, takže každá konfigurace jen *zrelaxuje do rovnováhy a zamrzne*
(žádné oběhy, oscilace, odraz při kolapsu). Lákalo přidat hybnostní pole
(2. řád), ale **zamítnuto**:

1. Je to cizí ontologii světa — **nehýbe se hmota, hýbe se energie/informace**.
   Hybnost by vyžadovala fyzickou polohu a setrvačnost entit, které svět nemá.
2. Globální pole rychlosti je drahé; chceme to udržet rozumně rychlé.

Zajímavost se tedy nebude brát z *pasivní* fyziky (oběhy), ale z **aktivní
práce programů** (viz dál). Gravitace zůstává difuzní silou prvního řádu.

## Zasazení do `tick.rs`

Gravitace i tlak vstupují jako **další členy do výpočtu per-směrové rate**, ne
jako separátní mechanika navěšená vedle. Stávající natural rate
(`compute_natural_rates`) počítá `stochastic_floor((E_self − E_nbr)·coeff)`
po směrech. Rozšíření:

```
M[c]        = Σ_{0<|d|≤R} E[c+d] / |d|          // gravitační potenciál (hmota)
Π(E)        = pressure · eref · (E/eref)^γ       // tlakový potenciál
drive[c→d]  = coeff·(E_c − E_nbr)                // radiace (z kopce dolů)
            + (Π(E_c) − Π(E_nbr))                // tlak (ven)
            + grav·(M_nbr − M_c)                 // gravitace (k hmotě, i do kopce)
rate[c→d]   = stochastic_floor(max(0, drive[c→d]))
```

Poté **stávající `proportional_clamp`** ořízne součet rate na `E_c` (nikdy
neodteče víc, než buňka má) — tím je zachována konzervace beze změny invariantu
`energy == mem_len`. Zbytek pipeline (pointer layout, outflow, inflow,
dominance) se **nemění**.

Akrece funguje přirozeně: hustá buňka má nejvyšší `M`, takže její gravitační
člen ven je záporný (drží energii); její řidší sousedé mají gravitační člen
*k ní* kladný (emitují do dolíku). Vše jako per-směrový outflow clampnutý na
dostupnou energii → konzervativní.

### Determinismus

`M`, `Π` i výsledné rate musí záviset jen na `(world_seed, tick, coord, d)`,
nikdy na pořadí iterace — stejně jako dnešní natural rate (`Rng::for_cell_at_tick`).
`stochastic_floor` použije týž seedovaný proud. Zachovat zamrzlou off-by-one
konvenci RNG (layout pro tick N se počítá na konci tick N−1).

## Hustotně vázaná mutace (překlápění bitů)

Zdroj novosti a evoluce. Návrhové principy:

- **Co se mutuje:** paměť = program = energie. Mutace = překlopení bitu ve
  slotu. **Nemění počet slotů**, jen jejich hodnotu → energie je automaticky
  konzervována, mutace se týká jen *obsahu/programu*.
- **Hustotní/gravitační coupling:** pravděpodobnost překlopení na slot za tick
  roste s lokální hustotou, resp. potenciálem:
  `p_flip(c) = base_rate · f(M_c)` (alternativně `f(E_c)`). Gravitační doly se
  tím stávají **mutagenními kotli evoluce** — gravitace pak není jen sochař, co
  hmotu sbírá, ale i mutagen. To dává gravitaci druhý smysl nad rámec
  „zpomalení vyzařování".
- **Determinismus zachován:** mutace se tahá ze seedovaného PRNG s klíčem
  `(world_seed, tick, coord, slot_index)`. Tím zůstává svět plně replayovatelný
  *a* zároveň se vyvíjí — „determinismus vs mutace" je falešné dilema.
- **Umístění v cyklu:** dedikovaná fáze nad sloty v aréně (kandidát: součást
  `end_of_tick`, po outflow/inflow, před GC). Aplikuje překlopení deterministicky
  per slot.

### Vztah k dominanci/intrusi

Intrusion model už při srážce **přepisuje** sloty cílové buňky přitékajícími
sloty (zápis od `write_start` podle dominance) — to je de facto **rekombinace**
(přepis kusu programu cizí energií). Bodová mutace je vrstva *navíc*. Otevřená
otázka: stačí implicitní rekombinace z toku energie, nebo je třeba i bodová
mutace? Pravděpodobně obojí, s laděním poměru.

## Zdroje novosti v celém systému

Engine je deterministický (seed + počáteční program ⇒ pevná trajektorie). Novost
má přesně tři zdroje, každý jiného druhu:

1. **Volba počátečního programu** — variabilita mezi vesmíry; nahrazuje umělý
   šum bohatšími, informaci nesoucími seedy z vykonávání asembleru.
2. **Hráčův program** — jediný exogenní vstup; systém je deterministický *mezi
   zásahy*. Hráčův program podléhá téže fyzice: aby přežil, musí konat práci
   (gravitaci může využít — nahustit energii v dolíku — nebo s ní bojovat).
3. **Mutace** — jediná dělá rozdíl mezi „vesmír se odvíjí" a „vesmír se vyvíjí".

Zajímavá dynamika tedy nepochází z parametrů fyziky, ale z vrstvy
**program × rekombinace × mutace × selekce** (kde selekci spoluurčuje i
gravitace). Fyzika (difuze + gravitace + tlak) je jen prostředí.

## Otevřené otázky

- **Cena dlouhodosahové gravitace v sparse/neohraničeném světě.** Naivně O(N²);
  potřeba cutoff `R` (O(N·R³)) nebo Barnes–Hut/strom (O(N log N)). Volba poklesu
  ovlivňuje cenu i chování (pomalejší pokles = delší dosah = sklon ke globálnímu
  kolapsu, dražší).
- **VM je reálné úzké hrdlo, ne gravitace.** `floor(E/K)` instrukcí v hustém
  třeskovém bodě je brutální; perf strop bude tady.
- **Void vs torus v produkci.** Sparse svět je otevřený (leak do voidu), takže
  struktury vznikají hůř než na toru prototypu 11. Zvážit, zda gravitační dosah
  a síla stačí, aby energii posbíraly dřív, než uteče.
- **Kalibrace rychlosti mutace (error threshold, Eigen).** Málo → žádná novost
  (zamrzne); moc → ztráta dědičnosti (struktury degradují rychleji, než se
  reprodukují). Zajímavá zóna je úzký pás — hlavní knob k ladění.
- **Coupling mutace:** na `E` (hustota) vs `M` (potenciál) vs kombinace.
- **Emergence není zaručena.** Z třeskového bodu (syrová energie, žádné entity
  v čase 0) je vznik sebeudržujících vzorů otevřená otázka (abiogeneze).
  Zhušťování instrukční sady je páka, ne parametr.

## Návaznost

Prototyp 11 zůstává jako referenční ověření fyziky — **VM se do něj nepřidává**
(zdvojovalo by `vm.rs`/`cpu_phase`). Další krok: až bude instrukční sada hustší,
napojit gravitaci/tlak přímo do `tick.rs` podle tohoto plánu a zkoumat emergenci
tam, kde už VM a `active_outflow` žijí.

## Stav implementace (2026-06-13)

✅ **Hotovo, brána zelená** (`./check` ALL GREEN + `cargo mutants` na nových
funkcích v `tick.rs` a na `snap_gamma`: 0 missed; dokumentované equivalent
mutanty ve `.cargo/mutants.toml`: `< → <=` u mutace a `+ → -` u offsetů v
`refresh_mass`, ten druhý je ekvivalentní symetrií stencilu).

- **Gravitace + tlak** vstupují jako členy do per-směrové rate v
  `compute_natural_rates` (`tick.rs`): `drive = coeff·(E_c−E_nbr) + (Π(E_c)−Π(E_nbr))
  + gravity·(M_nbr−M_c)`, pak stávající `proportional_clamp`. Konzervace i invariant
  `energy == mem_len` beze změny.
  - **Konfigurovatelný dosah `R` (`gravity_radius`, default 1).**
    `M(c) = gravity_alpha·Σ_{0<|d|≤R} E(c+d)/|d|` přes předpočítaný **stencil**
    (offsety + váhy `1/|d|`, jen `sqrt`/`/` ⇒ portable; pevné pořadí ⇒ determinismus).
    `R=1` = lokální (6 stěnových sousedů); `R>1` dává **skutečnou přitažlivost přes
    void** — energie se táhne dohromady přes vzdálenost (měřeno: počet buněk klesá
    R=1→10k, R=3→5k, R=6→3k při 60k energie; viz vizuál konsolidace). Cena `O(N·R³)`.
  - **`M` se počítá i ve voidu** (na „shellu" = stěnoví sousedi obsazených buněk),
    protože rate loop tam čte potenciál a void bod blízko vzdálené hmoty má reálný
    nenulový potenciál — to je nutné pro přitažlivost přes mezeru. `scratch_mass` je
    proto klíčován na occupied ∪ shell, aby rate loop mohl číst `scratch_mass`, zatímco
    mutuje `world.cells` (žádný borrow konflikt). Pozn.: tím se oproti úplně prvnímu
    R=1 milníku (void→0) změnilo void-suppression chování — legitimní vylepšení modelu,
    ne re-bless baseline (ty drží na `gravity=0`).
  - **Nulové defaulty = nulový re-bless.** `gravity=0 && pressure=0` bere zamrzlou
    fast-path (textově předgravitační kód) → všechny baseline (bit-parita,
    konzervace, determinismus) prošly **beze změny**. Aktivní cesta používá explicitní
    `if drive > 0.0`, nikdy `max(0,drive)` (jinak by ujídala RNG draw a desynchronizovala proud).
  - **UI default `R=3`** (sweet spot: nejvyšší koncentrace ~21 %, silná konsolidace,
    v sweepu nejnižší cena — méně buněk vyváží větší stencil).
- **Tlak `Π(E)=pressure·eref·(E/eref)^γ`** — γ omezeno na **portable hodnoty
  `{1, 1.5, 2, 2.5, 3}`** počítané přes `*`/`sqrt` (IEEE correctly-rounded ⇒
  bit-for-bit reprodukovatelné native↔wasm). Libovolné γ by vyžadovalo `powf`
  (není correctly-rounded, last-ULP drift native↔wasm by mohl přehodit
  `stochastic_floor` a rozejít dva světy) → mimo rozsah; `snap_gamma` ve WASM vrstvě
  zaokrouhlí vstup na nejbližší podporovanou hodnotu.
- **Hustotně vázaná mutace** — `apply_density_coupled_mutation` po `outflow_phase_inplace`,
  před `end_of_tick`. Per slot `p_flip = min(base_mutation_rate·E, 1)` (hustota = E,
  buňka = jednotkový objem ⇒ hustá jádra = mutagenní kotle), překlopení `slot ^= 1<<bit`
  mění hodnotu, ne počet slotů ⇒ energie konzervována. RNG: jeden proud na buňku,
  doména `DENSITY_MUTATION_RNG_DOMAIN=3` (disjunktní od 0/1/2), pozice slotu = pozice
  v proudu (žádná per-slot doména). `base_mutation_rate=0` ⇒ strict no-op (žádné RNG).
  Coupling na `E` (ne `M`) drží fázi self-contained; coupling na `M`/kombinaci je
  ladicí knob ponechaný otevřený.
- **Plumbing**: `SparseWorld` pole (`gravity`, `gravity_alpha`, `gravity_radius`,
  `pressure`, `pressure_gamma`, `pressure_eref`, `base_mutation_rate`, vše default
  0/neutral/R=1) → WASM settery/gettery → `protocol.ts`/`worker-state.ts`/
  `worker-handler.ts` → slidery v `index.html` + listenery v `web/main.ts`.

**Odloženo (beze změny plánu):** Barnes–Hut / strom (O(N log N) — cutoff `R` zatím
stačí, úzké hrdlo zůstává VM); libovolné γ přes `powf` (mimo reprodukovatelný režim);
coupling mutace na `M` vs kombinaci; native-server vrstva gravitace (zatím jen WASM viewer).
