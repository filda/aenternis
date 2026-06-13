# Aenternis — plán: procedurální genesis prvotní buňky (seedem řízený generátor programu)

Last updated: 2026-06-13 (návrh; opkódy + fold **hotové**, gravitace běží paralelně)

Designový kontext žije v `aenternis.md` (jádro, invarianty) a `vm.md` (instrukční
sada, model paměti/emise); obecný roadmap v `plan.md`; mutace a selekce, na které
tento plán navazuje, v `gravity-plan.md`; hustota instrukční sady, na které tento
plán **staví**, v `opcodes-plan.md` — ta je už **dodaná**: 31 opkódů (`0x00`–`0x1E`,
commit `f252784`) a totální `decode` přes `byte % COUNT` fold (každý byte je platná
instrukce). Gravitace/tlak/mutace (`gravity-plan.md`) se implementuje paralelně.

## Otázka, kterou plán odpovídá

Genesis dnes umí dva krajní režimy:

1. **Čistý šum** — `big_bang(seed, energy)` naplní paměť prvotní buňky
   deterministickým RNG (`xorshift32` z `origin_tag`). Po fold dekodéru (už
   dodaném) je šum **aktivní, ale chaotický** kód — každý byte je platná
   instrukce, ne nop. Variabilita mezi vesmíry je maximální, ale informační
   obsah nulový — emergence z čisté entropie je otevřená otázka (abiogeneze).
2. **Ruční program** — `big_bang_with_program(seed, energy, &program)` zapíše
   autorský prefix a zbytek dofiltruje šumem. Informačně bohaté, ale ne
   variabilní (jeden program, ne rodina vesmírů) a vyžaduje člověka.

Chybí **třetí cesta mezi nimi**: program **řízený seedem světa** — variabilní
jako šum, ale nesoucí strukturu jako ruční program. Přesně to popisuje
`gravity-plan.md` („Zdroje novosti", bod 1): *„volba počátečního programu…
nahrazuje umělý šum bohatšími, informaci nesoucími seedy z vykonávání
asembleru."* Tento dokument je realizace té věty.

Jádro nápadu (zadání): program nevzniká jako náhodný instrukční šum, ale
**skládáním drobných předvalidovaných stavebních kamenů — *maker* — z nichž každý
už sám o sobě něco dělá.** Náhodný bytový šum skoro nikdy nedělá nic koherentního
(házení scrabble kostek); makro má **zaručenou lokální funkci**, a generátor pak
prohledává prostor *kombinací slov*, ne *všech bitových vzorů*. To zvedá
pravděpodobnost emergence o řády a přitom drží seedovou rozmanitost.

## Rozhodnutí (shrnutí návrhové diskuse)

Čtyři rozcestí, vědomě uzavřená v tomto pořadí:

### R1 — Umístění: jádro (Rust), makra jako data

Kanonická genesis i **expander maker** žijí v `aenternis-core`. Důvody:

- Genesis je **artefakt světa**, ne UI feature → plně replayovatelná, engine umí
  seedovat vesmír sám, bez frontendu.
- **Dveře k pozdější vazbě mutace na seed** zůstávají otevřené (`gravity-plan.md`):
  pokud se mutace časem naváže na seedovou „genomovou pásku", musí to být tam, kde
  žije svět.

Klíčový důsledek: **makra jsou *data*, ne imperativní Rust kód.** Kdyby makro bylo
`fn emit_replicator(...)`, zvenku se nepoužije. Když je makro záznam „opkódy +
místa parametrů", dá se serializovat, sdílet a **napsat proti němu nezávislý
generátor**. To byl explicitní požadavek.

`src/asm.ts` **není zdroj pravdy** — je to UI pomocník pro freeform textareu.
Kanonická expanze maker je v Rustu; `asm.ts` může makra zrcadlit pro náhled, ale
nedefinuje je. (Stejný vztah, jaký dnes má `OPCODES` v `asm.ts` k `Opcode` enumu
v `vm.rs`, hlídaný paritním testem.)

### R2 — Rozsah: celá dostupná paměť, aperiodicky

Seed pokrývá **celou paměť** prvotní buňky, ne jen prefix. Důvod je **fyzika
emise**: defaultní pointery sedí na *konci* paměti (`vm.md`: `xp_ptr =
memSize − Σrate`), takže pasivní emise kopíruje sloty z **vysokých adres
(membrány)**. Kdyby byl rozumný program jen prefix na nízkých adresách a zbytek
šum, potomci by **pasivní cestou dědili přesně ten šum**. „Celá paměť = rozumný
kód" tuhle závislost ruší: ať teče ven cokoliv a odkudkoliv, nese to smysluplný
program → fragmenty rodiče se okamžitě dostanou do potomků/nových entit.

- **Cena = jako dnešní šum.** Genesis stejně plní *každý* slot. Expander jen místo
  `noise_rng.next_u32()` „odvíjí proud maker" — jeden `O(energy)` průchod, jedna
  alokace. 1 M slotů smysluplného kódu **není** jiná váhová kategorie. Runtime
  cena se nemění vůbec (CPU vykoná `floor(E/K)` instrukcí tak jako tak; pod foldem
  je i šum aktivní).
- **Aperiodicky**, ne dlážděně. Emise ukrajuje z proudu náhodné plátky — občas
  celý sub-organismus, občas půlka instrukce. **„Rozbité" entity nejsou vada, jsou
  předpoklad:** bez částečných/poškozených fragmentů není variace, bez variace
  selekce. Fold dekodér zajistí, že i „půlka instrukce" je platný (jen jiný) kód,
  takže nic nespadne. (Periodický/dlážděný režim — zaručené dědění celého
  programu — viz „Mimo rozsah" jako pozdější laditelný extrém.)

### R3 — Viabilita: žádná zvláštní mechanika, jen váhy

Životaschopnost **není** samostatný mechanismus (žádná `if guarantee {
force_replicator() }` větev). Je to **statistika vah** v knihovně maker: když je
v poolu replikátor (`setp`/`port`) s nenulovou váhou, občas se ze seedu vylosuje
a vesmír se začne šířit; když ne, je „mrtvý". To přesně sedí na R2 („rozbité je
předpoklad"). „Plodnost" světa = váha šíř-tvorných maker, vystavená jako config
s neutrálním defaultem. **Viabilita se tím slévá do R4 (knihovna + váhy).**

### R4 — Kompozice (v1): vážený stream rámovaný jako genotyp→fenotyp

v1 = **losuj makro dle vah → dosaď parametry ze seedu → expanduj na sloty →
připoj → opakuj, dokud není paměť plná.** Přímočaré, robustní, triviálně
testovatelné.

Pointa rámování: u aperiodického streamu je **„vážený stream" a „genotyp→dekodér"
prakticky totéž** — seedový RNG proud *je* genomová páska a „vyber vážené makro"
*je* dekódovací krok. Když to takhle pojmenujeme, máme **genotypový rámec
zadarmo** (důležitý pro pozdější vazbu mutace na seed), při složitosti plochého
streamu. Gramatika / L-systém (vnořená makra) je hlavní zdroj degenerace a těžko
se ladí → **až vylepšení, ne v1** (viz „Mimo rozsah").

## Architektura — dvě vrstvy

```
makra (data)            ──┐
  parametrizované          │  expander (Rust, kanon)   ──┐
  assembler fragmenty      │    fragment + params → sloty │  generátor (Rust)
                         ──┘                             ──┘    seed → program
                                                                  pro celou paměť
            ▲
            └── zrcadlo / náhled v asm.ts (UI), nezávislé generátory zvenku
```

- **Knihovna maker** — sdílený, vystavený artefakt. Deklarativní.
- **Expander** — přeloží jedno makro (s dosazenými parametry) na `Vec<u32>`.
  Kanonicky v Rustu (R1). Je to **malá podmnožina assembleru** (mnemonika +
  operand parsing + placeholdery); tabulku mnemonik už máme (`Opcode`), operand
  parsing je pár řádků. Druhá (UI) implementace v `asm.ts` se hlídá paritním
  testem na pevné sadě maker.
- **Generátor** — algoritmus skládající makra do programu ze seedu. Jádro má
  jednu kanonickou/výchozí instanci; nezávislý generátor je jen *jiný konzument
  téže knihovny*.

### Formát makra (A4, rozhodnuto)

**Metadata v `;`-komentářích, tělo = čistý assembler** (stejný jazyk jako presety).
Jeden soubor `crates/aenternis-core/macros/v1.aenm`, embedovaný `include_str!`,
parsovaný jednou. Makra oddělená hlavičkou `; @macro`:

```
; v1 macro library — bezskoková dataflow makra pro seedovanou genesi.
; Formát:  ; @macro NÁZEV weight=W tags=T1,T2
;          ; @param NÁZEV TYP          (TYP ∈ DIR | ADDR | CONST)
;          <tělo: assembler, díry {param}, ve v1 bez skoků a labelů>

; @macro xor weight=5 tags=bit
; @param a ADDR
; @param b ADDR
  xor {a}, {b}

; @macro set weight=5 tags=data
; @param a ADDR
; @param v CONST
  set {a}, {v}

; @macro replicate weight=3 tags=spread
; @param d DIR
; @param src ADDR
  setp {d}, {src}

; @macro sense_mix weight=2 tags=sense
; @param d DIR
; @param a ADDR
; @param b ADDR
  senergy {d}, {a}
  add {b}, {a}
```

- **Tělo je skoro validní program** — s dosazenými parametry jde zkopírovat do
  textarey a spustit / ověřit „sám něco dělá". Existující comment-stripper v
  `asm.ts` metadata ignoruje. Drží to „makro = mrňavý preset", žádný cizí formát.
- **Parser triviální:** `; @macro` zahájí makro, `; @param` přidá parametr, ostatní
  řádky jsou tělo.
- **v1 typy parametrů: jen `DIR` (0..DIRS−1), `ADDR` (modulární adresace dělá
  *jakoukoli* hodnotu bezpečnou), `CONST` (libovolný u32).** Sémantiku vzorkování
  z pásky definuje A5. **Labely a `HERE` → v2** — bezskokovost je činí zbytečnými
  (replikátor `setp dir,{addr}` funguje s libovolnou adresou, protože celá paměť je
  kód, R2; `HERE` je užitečné až s dopřednými skoky).
- **Multi-line composite makra povolena** (`sense_mix` výše) — **nositel lokálního
  chování**; makro není jen 1 instrukce.
- **Žádný globální `loop:` spine ani skok-makra** — viz „Řídicí tok" níže; v1 makra
  jsou bezskoková (rovná), opakování zařídí tick + PC-wrap.
- **Váha** řídí pravděpodobnost losu; **tagy** (`spread`, `clock`, `bit`, …) jsou
  pro laditelné přeskupení vah (R3 „plodnost" = váhový multiplikátor tagu
  `spread`).

### Řídicí tok — tick *je* smyčka (A3, rozhodnuto)

Při K=1 je `instrukcí = energy = memSize`, takže PC za jeden tick proběhne (zhruba)
**celou paměť** a na konci se zabalí na 0 (`vm.md`). **Opakování tedy zařídí hranice
ticku + PC-wrap — žádná globální páteř ani `jmp loop` není potřeba.** Presety
používaly `jmp loop`, protože to byly malé ruční programy; v whole-memory genesis je
to zbytečné (replikátor `setp dir,{addr}` je rovná instrukce, override se znovu
nastaví každý tick, jak jím PC proteče).

Z toho plyne **v1 = bezskokový dataflow genom**:

- **`jmp` ven nadobro** — skok na seedem náhodnou adresu uvězní PC v malé smyčce →
  degenerovaný „spinner", co protočí pár slotů `floor(E/K)`× a zbytek paměti nikdy
  nespustí. To zabíjí smysl „celá paměť je aktivní".
- **Podmíněné skoky (`jz/jne/je/jp/jn`) odloženy do v2** — absolutní cíl mod memSize
  je rovněž past. Datová závislost přitom *nemizí*: nese ji **nepřímá adresace**
  (`ldi`/`sti` — adresa závisí na datech ⇒ chování závisí na datech, bez větvení).
- Skoky stejně vstoupí do populace **mutací** (překlopení bitu vyrobí skok-opkód) a
  hráčovými programy. Evoluční příběh: genesis zaseje bezskokové dataflow organismy,
  evoluce teprve objeví řídicí tok. v2 první krok = *omezený dopředný* podmíněný skok
  (cíl = offset + malá konstanta, nemůže uvěznit PC).

### Taxonomie maker — v1 (rozhodnuto)

Každé makro „sám něco dělá" a (v1) je **bezskokové**. Existující `PRESETS`
(counter, self_xp, …) se přirozeně stávají makry — ale jejich `jmp loop` v genesi
odpadá (tick je smyčka), takže např. `counter` = pouhé `inc {a}`.

Verdikt a váhová úroveň pro v1 (HIGH > MED > LOW; OUT = negeneruje se):

| Rodina | Makra | v1 | Váha | Tag | Proč |
|--------|-------|----|----|----|------|
| **Bitové gadgety** | `and/or/xor/not/shl/shr` | ✅ | **HIGH** | `bit` | nejvyšší hodnota pro emergenci — syntéza adres/opkódů z bitů, hladká fitness krajina |
| **set (immediate)** | `set {a},{v}` | ✅ | HIGH | `data` | zasadí konstanty/pracovní data |
| **Replikátory** | `setp {dir},{addr}` | ✅ | MED | `spread` | jediný způsob šíření programu; bezskokový |
| **Igniter** | `port {dir},{i}` | ✅ | MED | `spread` | aktivní outflow → pohyb/boj/zážeh |
| **Čítač** | `inc {a}` / `dec {a}` | ✅ | MED | `clock` | levný vnitřní stav, „čas" |
| **Aritmetika** | `add/sub {a},{b}` | ✅ | MED | `arith` | kombinace slotů |
| **Kopírky** | `copy {a},{b}` | ✅ | MED | `copy` | přesun dat |
| **Senzor** | `senergy {dir},{a}` | ✅ | MED | `sense` | jediný vstup z okolí; uloží energii souseda do paměti |
| **Barva** | `paint {v}` | ✅ | MED | `mark` | **lineage je vidět ve viewru** — pozorování emergence |
| **Aritmetika (exotická)** | `mul/div/mod {a},{b}` | ✅ | LOW | `arith` | div/mod nulou = 0 (bezpečné) |
| **Nepřímá adresace** | `ldi/sti {a},{b}` | ✅ | LOW | `indirect` | datová závislost **bez větví** (náhrada za skoky), self-mod |
| **Identita** | `sid {a}` | ✅ | LOW | `mark` | vlastní origin_tag do paměti (self-recognition/quine) |
| **Pointer-read** | `getp {dir},{a}` | ✅ | LOW | `ptr` | čtení vlastního layoutu (introspekce vlastní = OK) |
| **setpv** | `setpv {dir},{a}` | ⛔→v2 | — | — | runtime-počítaný setp; pokročilé |
| **Skoky** | `jmp` | ⛔ | — | — | uvězní PC → degenerovaný spinner |
| | `jz/jne/je/jp/jn` | ⛔→v2 | — | — | absolutní cíl = past; vstoupí mutací; v2 = omezený dopředný |
| **nop** | `nop` | ⛔ | — | — | přesně to, čemu se vyhýbáme oproti šumu |

**Bitové gadgety + `set` na HIGH** je záměr: tam žije „syntéza hodnot z bitů" i
hladká fitness krajina pro mutaci. **`spread` tag** (setp+port) je „plodnost" knob
(R3/A2). Životaschopný zárodek = replikátor + čítač + senzor zapsaný do paměti, co
pak krmí port přes aritmetiku/nepřímou adresaci (bez větvení).

### Generátor v1 (algoritmus)

```
genesis(seed, energy) -> Vec<u32> délky přesně `energy`:
    tape = Rng::new(cell_seed(seed, ORIGIN))     // genomová páska (= dnešní šum RNG)
    out  = Vec s kapacitou energy
    while out.len() < energy:
        m      = vyber_vážené_makro(tape, cfg)    // draw % Σweight → váhový kbelík
        params = navzorkuj_parametry(tape, m, cfg)// viz A5 níže
        slots  = expand(m, params)                 // fragment → Vec<u32>
        připoj slots do out, ořízni poslední makro na `energy`  // částečný plátek = OK (R2)
    return out
```

- **Determinismus:** čistá funkce `(seed, energy, cfg)`. Stejný seed → stejná paměť.
- **Konzervace:** generátor jen plní *počáteční* paměť; `energy == mem_len`
  triviálně platí (stejně jako u dnešního šumu). Žádný runtime invariant se
  nedotýká.
- **Bezpečnost vůči VM:** každé makro expanduje na platné instrukce; pod fold
  dekodérem je navíc *každý* byte platný, takže i oříznutý poslední fragment je
  bezpečný.

### Vzorkování ze seed-pásky (A5, rozhodnuto)

**Páska** = posloupnost u32 z `Rng::new(cell_seed(seed, ORIGIN))` (týž PRNG, co dnes
plní šum). Generátor ji čte zleva doprava; každé rozhodnutí utrhne **jednu u32**
(žádné bit-packing — páska se po genesi zahodí, runtime mutace jede na slotech, ne
na pásce ⇒ hustota kódování nehraje roli). Mapování `draw → hodnota`:

| Rozhodnutí | Mapování |
|------------|----------|
| výběr makra | `draw % Σweight` → váhový kbelík (váhy `spread`-maker škálované „plodností") |
| `DIR` | `draw % DIRS` |
| `ADDR` | `draw % A` |
| `CONST` | `draw` (uniform u32) |

**Klíč je `A` — velikost sdíleného pracovního okna, default `A = 256` (absolutně).**
`ADDR % memSize` (memSize ~1 M) by bylo **sterilní**: dvě makra by skoro nikdy
nesáhla na týž slot → žádná interakce, jen nezávislé čmárání. Okno `[0, A)` dělá
sdílený „registr file": ~1 M slotů kódu pracuje nad společnou ~256slotovou oblastí
→ teprve to umožní výpočet. Že okno leží na nízkých adresách, kde je i kód, znamená
**self-modifying soup** — vítaná emergentní vlastnost (fold drží vše platné), v1 se
neřeší.

**`A`, váhy, „plodnost", rozdělení `CONST` jsou všechno knoby *generátoru* (`cfg`),
ne vlastnosti VM ani maker.** Makro deklaruje jen *typ* (`ADDR`) a zůstává
přenositelné; VM zná jen modulární adresaci přes memSize. Jiný generátor smí
vzorkovat ze stejné knihovny úplně jinak. Společná ladicí plocha emergence:

| Knob (`cfg`) | Řídí |
|--------------|------|
| váhy maker | četnost rodin v proudu |
| „plodnost" (×`spread`) | jak často replikátor/port → šíření |
| okno `A` (default 256) | velikost sdíleného pracovního setu → míra interakce/výpočtu |
| granularita pásky | fixně 1 u32 / rozhodnutí |
| rozdělení `CONST` | uniform u32 |

### Zasazení do `big_bang`

Dnešní API má tři režimy fillu schované za dvěma funkcemi. Návrh je sjednotit do
**explicitního režimu genesis**:

**Dvě ortogonální osy** (A6, rozhodnuto), ne tři enum varianty:

```rust
enum Base { Noise, Macros }            // čím se naplní CELÁ paměť (default Macros)
struct Genesis<'a> { base: Base, overlay: Option<&'a [u32]> }  // overlay = hráčův prefix
```

Base se vygeneruje pro `[0, energy)`, **hráčův `overlay` se zapíše od adresy 0 přes
`[0, len)`, zbytek (base) zůstává nedotčený.** Matice pokrývá dnešek i nový default:

| base | overlay | = |
|------|---------|---|
| Noise | — | dnešní `big_bang` (baseline/testy) |
| Noise | prefix | dnešní `big_bang_with_program` |
| **Macros** | — | **nový default světa** (Reset, WASM `World.new`) |
| **Macros** | prefix | **doporučený hráčův režim** |

- **Default base = `Macros`** — nahrazuje umělý šum bohatším startem
  (`gravity-plan.md`); `Noise` zůstává jako legacy/test toggle (analogie
  `step_diffusion` vedle `step`).
- **Overlay nahrazuje, nespotřebovává pásku** (jako dnes RNG): ocas `[len, energy)`
  je byte-identický bez ohledu na prefix → zachová „porovnatelné pozadí".
- **Macro ocas i pod hráčem** (R2 znovu): pasivní emise jde z konce paměti; se
  šumovým ocasem by replikátor v prefixu šířil šum, s makro ocasem rozumný kód.
- **Hráč makra *needituje přes generátor*** — jen si je ručně zkopíruje do zdrojáku
  (jako presety). Knihovna maker tak pohání **dva nezávislé konzumenty**: generátor
  (base fill) a UI „vkládač snippetů" (rozšíří `PRESETS` dropdown).

## Vztah k foldu, mutaci a evoluci (proč to celé drží pohromadě)

Tři vrstvy z `opcodes-plan.md` a `gravity-plan.md` do sebe zapadají právě skrz
makro-genesis:

1. **Fold dekodér** (`% COUNT`, **už dodaný**, `COUNT = 31`) → každý byte je
   platná instrukce → emitovaný *částečný* fragment makra je platný (jen jiný)
   kód. Bez foldu by „rozbité" entity z R2 byly z velké části nop.
2. **Bodová mutace** (`gravity-plan.md`) překlopí bit ve slotu → pod foldem skoro
   vždy vyrobí *jinou platnou instrukci*, často téhož makra. Makro-genesis tím
   dává **hladkou fitness krajinu**: malé mutace = malé změny chování, ne pád do
   nopového šumu. To je hlavní „proč" celého přístupu oproti čisté-šumové genesi.
3. **Genom = fenotyp.** Sloty jsou *zároveň* genom i fenotyp — replikují se
   verbatim a běží přímo. **Makro-genesis je čistě abiogenezní krok** (seed →
   počáteční sloty); runtime evoluce pak jede na syrových slotech přes
   mutaci/rekombinaci. Genotypový rámec z R4 platí jen *při genesi* — žádný
   „genome→compile" za běhu. Tuhle hranici držet jasně.

## Synchronizační body (kam sáhnout)

1. **`crates/aenternis-core/`**
   - nový modul (kandidát `genesis.rs` + `macros.rs`): expander podmnožiny
     assembleru, parser formátu makra, generátor v1.
   - knihovna maker jako data — kandidát `crates/aenternis-core/macros/*.macro`
     embedované přes `include_str!` a parsované jednou (líně/při startu), nebo
     jeden `macros.macro`. (Formát = R4 výše.)
   - `Genesis` enum + napojení do `big_bang` (sjednocení tří režimů).
2. **`crates/aenternis-wasm/`** — vystavit `Genesis::Macros` (default `World.new`)
   a volitelně knihovnu maker pro UI náhled.
3. **`src/asm.ts`** — UI zrcadlo expanze maker (náhled v textaree); paritní test
   proti Rustu. **Není zdroj pravdy.**
4. **`web/` frontend** — preset dropdown rozšířit/nahradit: „generovaný program ze
   seedu" jako default; zobrazit seed; náhled vygenerovaného programu.
5. **`src/presets.ts`** — migrovat na makra (nebo nechat jako kompozice maker).
6. **Dokumentace** — `vm.md` (sekce o genesi/fillu), `plan.md` (status), případně
   `aenternis.md` (novost #1 realizovaná).

## Testy / brána kvality

Brána = zelený `./check` (rustfmt + clippy -D warnings + cargo test +
vitest/coverage + WASM build) **a** `cargo mutants` bez nových mezer (genesis je
algoritmická změna → mutační testování je součást brány).

- **Determinismus:** stejný `(seed, energy)` → byte-identická paměť; dva seedy →
  různá.
- **Délka:** generátor naplní *přesně* `energy` slotů (poslední makro oříznuto).
- **Platnost:** každé makro v knihovně expanduje na sloty, které dekódují na
  definované opkódy (a po foldu triviálně i bez toho).
- **Parita Rust ↔ `asm.ts`:** pevná sada maker expanduje na shodné sloty.
- **Konzervace/invarianty:** `energy == mem_len` po genesi; existující invariantní
  testy (konzervace, determinismus, `world.len() ≤ E_total`, bit-parita rayon)
  zůstávají zelené beze změny — genesis mění jen *počáteční* obsah.
- **Statistika viability (volitelné, ne-flaky):** přes vzorek seedů ověřit, že při
  defaultních vahách nenulový zlomek vesmírů obsahuje replikátor — ladicí metrika,
  ne tvrdý assert.

## Otevřené otázky

- **Kalibrace vah** je hlavní ladicí páka emergence (analogie „pořadí opkódů" v
  `opcodes-plan.md`). Příliš málo `spread` → mrtvé vesmíry; příliš mnoho → jen
  triviální replikátory bez vnitřní práce. Zajímavá zóna je úzký pás.
- **Velikost okna `A`** (A5, default 256) je — vedle vah — hlavní empirická páka:
  malé okno = chaotický stomp, velké = řidší interakce. Ladit spolu s vahami.
- **Cross-verze.** Změna knihovny maker / vah mění programy pro daný seed — stejný
  caveat jako verzování ISA (`plan.md` 2026-05-13: replay platí *uvnitř* verze).
  Zdokumentovat „verzi genesis".
- **Více seedovaných buněk.** Zatím jedna prvotní buňka (zadání). Seedovat rovnou
  několik buněk různými programy (startovní ekosystém)? Follow-up.
- **Vazba mutace na seedovou pásku.** Dveře otevřené (R1); konkrétní mechanika je
  na `gravity-plan.md`, ne tady.

## Mimo rozsah (zatím)

- **Gramatika / L-systém / hierarchická makra.** Hlavní zdroj degenerace; přidat
  jako vylepšení nad v1, až bude vážený stream zelený a odladěný.
- **Periodický / dlážděný režim.** Zaručené dědění *celého* programu v každém
  emitovaném plátku (perioda P). Vědomě odloženo ve prospěch aperiodicity (R2);
  je to jen druhý konec téhož „period" knobu (aperiodické = P→∞).
- **Mutační operátory nad genomem.** To je `gravity-plan.md` (hustotně vázaná
  mutace), ne genesis. Genesis jen vyrobí seedovaný start a nechá dveře otevřené.
- **Skoky a `setpv` v genesi (→ v2).** v1 je bezskokový dataflow genom (viz „Řídicí
  tok"). První v2 krok = *omezený dopředný* podmíněný skok (cíl = offset + malá
  konstanta, nemůže uvěznit PC); `jmp` zůstává ven nadobro. `setpv` (runtime setp)
  taktéž v2.

## Stav implementace

**Increment 1 — Rust jádro (hotovo, `./check` zelený + `cargo mutants` bez mezer):**

- `vm.rs`: `Opcode::mnemonic` / `from_mnemonic` / `arg_count` (zdroj pravdy pro expander).
- `src/macros/v1.aenm`: knihovna ~24 maker dle taxonomie v1 (bezskoková).
- `src/macros.rs`: parser formátu (`; @macro`/`; @param`/tělo) + kanonický expander
  (podmnožina assembleru), `library()` přes `OnceLock`.
- `src/genesis.rs`: `GenesisConfig { window=256, fertility=1.0 }` + `generate_into`
  (vážený stream přes celou paměť, vzorkování ze seed-pásky dle A5).
- `world/sparse.rs`: `Base { Noise, Macros }` + `big_bang_with(seed, energy, base,
  overlay)`; `big_bang` / `big_bang_with_program` jsou tenké wrappery (zachované
  chování), nové `big_bang_macros`.
- Testy: unit (macros + genesis) + `tests/genesis.rs` (determinismus, celá paměť,
  konzervace, overlay tail-identity, zpětná kompatibilita šumu).

**Increment 2 — integrace (zbývá):** WASM binding pro `Base::Macros` jako default
`World.new`; TS parita expanderu + UI „vkládač snippetů" místo/vedle `PRESETS`
(makra = dva konzumenti, A6). Jádro je samostatně kompletní a otestované.

## Návaznost

Genesis **staví na** `opcodes-plan.md` (hustá sada + fold — bez nich nejsou makra
expresivní a fragmenty nejsou platné) a **krmí** `gravity-plan.md` (seedovaný
informaci-nesoucí start je palivo pro selekci v gravitačních dolech a substrát pro
mutaci). Předpoklad „hustá sada + fold" je **splněn** (commit `f252784`), takže
v1 makro-genesis může jít hned. Gravitace/mutace se dodává paralelně; až bude
v jádře, genesis jí dodá informaci-nesoucí start místo šumu — pořadí mezi nimi
není kritické, jsou nezávislé až po bod, kde se mutace případně naváže na
seedovou pásku (R1, otevřená otázka).
