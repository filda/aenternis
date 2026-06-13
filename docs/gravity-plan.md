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
- **Rozptyl je úskalí, ne ztráta energie.** Pozor na terminologii: prototyp 11 byl
  *konečná* mřížka, kde energie na voidové hranici opravdu opouštěla simulaci
  (`sum + leaked = E_total`). **Produkční sparse svět je ale neohraničený a energii
  zachovává přesně** — `total_energy` je konstantní invariant (testováno každý tick).
  Nic „nemizí" a nikam „neutíká". Skutečný problém je **rozptyl/ředění**: difuze
  rozprostírá energii do stále většího objemu, takže hustota všude klesá a struktury
  se nemají z čeho tvořit. Gravitace proti tomu energii sbírá do shluků. (Torus i
  „leak" byly jen prototypové pomůcky konečné mřížky a dál se neuvažují.)
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
- **Rozptyl vs koncentrace v neohraničeném světě.** Energie je zachována (nic
  neutíká, `total_energy` konstantní), ale difuze ji ředí do rostoucího objemu.
  Otázka: stačí gravitační dosah a síla, aby energii sbíraly do struktur rychleji,
  než ji difuze rozptýlí na tenký rovnoměrný oblak? (Empiricky ano — koncentrace
  ~2–3× nad čistou difuzí; ladění parametrů to má posílit.)
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
- **Hustotně vázaná mutace** — ✅ implementováno (2026-06-13, po genesi).
  `apply_density_coupled_mutation` po `outflow_phase_inplace`, před `end_of_tick`.
  Per slot `p_flip = mutation_strength · E/(E+K)` (saturující, ne lineární),
  překlopení `slot ^= 1<<bit` mění hodnotu, ne počet slotů ⇒ energie konzervována.
  RNG: jeden proud na buňku, doména `DENSITY_MUTATION_RNG_DOMAIN=3` (disjunktní od
  0/1/2), pozice slotu = pozice v proudu. `mutation_strength=0` ⇒ strict no-op
  (žádné RNG) → zero-default drží.
  - **Couplováno na `E`** (lokální hustota = hmota), ne na `M` (potenciál).
  - **Dva parametry:** `mutation_strength` (0..1, default 0 = off, = strop/intenzita;
    `strength=1` → jádro až ~100 %) a `mutation_half_density = K` (default 40 000 =
    hustota, kde `p` dosáhne `strength/2`; `K=0` → `p=strength` nezávisle na hustotě).
    Tvar: `E≪K`→~0 (1slotový program nic nedělá), roste, `E≫K`→~strength.
  - **`K` vysoké** (nad hráčovou entitou ~tisíce E ⇒ jemné) ⇒ silně mutuje až
    gravitací nahuštěné husté jádro. `K≈40 000` je startovní odhad; **přesnou hodnotu
    nakalibrovat experimentálně** až poběží selekce (Eigenův práh). Důsledek:
    **gravitace rozhoduje, KDE se evoluce děje** (klidná periferie, husté doly =
    mutagenní kotle).
  - **1 bit/slot/tick** (hladká fitness krajina pod foldem); i `p=100 %` je „každý
    slot −1 bit z 32", plné zrandomizování až po ~desítkách ticků. Pořadí
    geneze→vyzáření→mutace dá genesis čistý první běh + emisi.
  - Všechny ops `+`/`*`/`/` correctly-rounded ⇒ reprodukovatelné native↔wasm.
    `cargo mutants` na funkci: 16/16 caught (+ dokumentovaný equivalent `< → <=`).
- **Plumbing**: `SparseWorld` pole (`gravity`, `gravity_alpha`, `gravity_radius`,
  `pressure`, `pressure_gamma`, `pressure_eref`, `mutation_strength`,
  `mutation_half_density`, vše default 0/neutral/R=1) → WASM settery/gettery →
  `protocol.ts`/`worker-state.ts`/`worker-handler.ts` → slidery v `index.html` +
  listenery v `web/main.ts`.

**Odloženo (beze změny plánu):** Barnes–Hut / strom (O(N log N) — cutoff `R` zatím
stačí, úzké hrdlo zůstává VM); libovolné γ přes `powf` (mimo reprodukovatelný režim);
coupling mutace na `M` vs kombinaci; native-server vrstva gravitace (zatím jen WASM viewer).

## Kalibrace mutace (2026-06-13)

Headless kalibrace celého systému (genesis `big_bang_macros` + gravitace + tlak +
mutace; obrázky v `reports/gravity-vis/calib-*`). Zjištění:

- **Celý systém funguje pohromadě:** genesis program se šíří (1 buňka → tisíce),
  gravitace drží strukturu, mutace ji drží naživu. Bez mutace `max_E` zamrzne
  (přetlumená rovnováha); s mutací jádro churnuje (nezamrzá).
- **`K` je čistý tradeoff živost ↔ jemnost hráče, na všech škálách.** Důvod
  (nečekaný): **mutace sama brání jádrům zhustnout** — bez mutace jádro dorostlo
  na ~11 k E (při 300 k energie), se zapnutou mutací se drží ~1,2 k (tug-of-war:
  gravitace hustí, mutace rozbíjí). Jádra tedy zůstávají na hráčské úrovni a
  separaci „husté jádro horké / hráč jemný" **přes `K` samotné nelze** dosáhnout.
  Pravá „mutagenní kotle" by chtěla gravitaci tak silnou, aby jádra zhustila
  navzdory mutaci — hlubší gravitační režim, **odloženo**.
- **Zvoleno (viewer default): `mutation_strength = 1`, `K = 15 000`** — živé
  (peak churn cv≈0,20), hráčova ~tisíce-E entita ještě přežije (`p≈0,17`).
  Struktura (top1% ~14 %) drží napříč `K` (gravitace měkké jádro udrží).
  Alternativy: `K=6000` (nejživější, hráč se vaří `p≈0,33`), `K=40000` (jemné,
  málo živé). Engine default zůstává `strength=0` (off); `K=15000` je jen
  viewer-config default.
- **Genesis napojena do vieweru (2026-06-13):** WASM `World::new` →
  `big_bang_macros`, `World::newWithProgram` → `big_bang_with(Base::Macros, program)`
  (makro-genesis + volitelný hráčův prefix). Viewer tedy běží na makro-genesi;
  `Base::Noise` zůstává v core API pro baseline/testy. `GenesisConfig` (window,
  fertility) zatím na defaultu — do UI nevystaveno.
- **Pozn.:** gravitační hodnoty samotné (`g=0.12/R3/pressure0.2/eref8`) zatím
  neladěny — samostatný krok.

## Ladicí poznámka: víc center = fluktuace × gravitace (2026-06-13)

Headless sonda (šumové pole — **jen testovací nástroj**, produkční start je **vždy
big-bang**) potvrdila roli gravitace jako **zesilovače density-perturbací**:

- **Bez perturbací** (hladký jednobodový big-bang mrak) → gravitace dá **jedno
  centrum** (mrak se jen zhušťuje). Mapa rovnováhy záření↔gravitace: gravitace je
  hlavní knob škály (slabá → měkký mrak, silná → těsná kapka), radiace `coeff`
  sekundárně rozšiřuje. Koncentrace top1% saturuje (~20 %, clamp), rozlišuje počet
  buněk (konsolidace).
- **S perturbacemi** → **fragmentace na víc center** (gravitační nestabilita).
  Režim z fragmentace: silná gravitace (g≈1.6+), **malé R=2** (jemná škála — menší R
  = víc menších center), slabá radiace (coeff≈0.1), nízký tlak; pole velké vůči R.

**Závěr pro produkci:** perturbace nemá dodávat umělý šum, ale **genesis (bohatý
seedovaný počáteční program) + mutace** — analogie **fluktuací v reliktním záření**,
které gravitace zesílí do struktury. Pořadí prací: **nejdřív genesis + mutace**
(zdroj fluktuací), **pak kalibrace gravitačních hodnot** k big-bang startu (teď
předčasné — závisí na velikosti perturbací). Obrázky: `reports/gravity-vis/`.
