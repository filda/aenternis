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

### Formát makra

Assembler syntaxe (stejný jazyk jako dnešní presety) + tenká vrstva: **deklarace
parametrů, váha, tagy, placeholdery.** Návrh:

```
; macro: replicator      weight: 3      tags: spread
; param dir  : DIR                      ; 0..DIRS-1
; param home : ADDR                     ; adresa v paměti
  setp {dir}, {home}
```

- **Typy parametrů:** `DIR` (směr 0..DIRS−1), `ADDR` (adresa; modulární adresace
  dělá *jakoukoli* hodnotu bezpečnou), `CONST` (libovolný u32). Případně `HERE`
  (offset začátku tohoto makra v globálním programu — pro self-referenci
  replikátoru, generátor ho zná).
- **Lokální labely** uvnitř makra; generátor je při expanzi posune o aktuální
  globální offset. Žádný globální `loop:` spine není potřeba — PC se po dosažení
  `memSize` zabalí na 0 (`vm.md`), takže *wrap sám je smyčka*; při K=1 PC stejně
  proběhne skoro celou paměť za tick. Skok-makra dělají vnitřní strukturu.
- **Váha** řídí pravděpodobnost losu; **tagy** (`spread`, `clock`, `bit`, …) jsou
  pro laditelné přeskupení vah (R3 „plodnost" = váhový multiplikátor tagu
  `spread`).

### Taxonomie maker (návrh k ořezání)

Každé makro „sám něco dělá". Existující `PRESETS` (counter, self_xp, self_omni,
beacon, quine_core, projectile) se přirozeně stávají makry s fixními/parametrickými
hodnotami — kontrola, že formát je dost expresivní.

| Rodina | Makra (opkódy) | Role | Tag |
|--------|----------------|------|-----|
| **Replikátory** | `setp {dir}, {home}` | drží core ve směru → šíření | `spread` |
| **Igniter / projektil** | `port {dir}, {i}` | aktivní outflow → pohyb/boj | `spread` |
| **Hodiny / čítač** | `inc {a}` / `dec {a}` | vnitřní stav, čas | `clock` |
| **Aritmetika** | `add/sub/mul {a},{b}` | kombinace slotů | `arith` |
| **Bitové gadgety** | `xor/and/or/shl/shr/not` | **syntéza adres/opkódů z bitů** | `bit` |
| **Kopírky** | `copy {a},{b}`, `ldi/sti` | přesun dat, self-reference | `copy` |
| **Senzor-reakce** | `senergy {dir},{a}` + gated `port` | reakce na okolí (predátor) | `sense` |
| **Identita / barva** | `sid {a}`, `paint {v}` | lineage/vzhled — **vidět ve viewru** | `mark` |
| **Pointer-math** | `getp`/`setpv` | čtení layoutu, výpočet reprodukce | `ptr` |

Životaschopný „zárodek organismu" = replikátor + čítač + senzorem gatovaný port.
**Bitové gadgety mají nejvyšší hodnotu pro emergenci** (`opcodes-plan.md`): dovolí
programu *syntetizovat* hodnoty, adresy i opkódy z bitů — to je přesně, co
umožňuje vznik smysluplného kódu z kombinací.

### Generátor v1 (algoritmus)

```
genesis(seed, energy) -> Vec<u32> délky přesně `energy`:
    tape = Rng::new(cell_seed(seed, ORIGIN))     // genomová páska
    out  = Vec s kapacitou energy
    while out.len() < energy:
        m      = vyber_vážené_makro(tape, library)        // dekód: tag váhy → makro
        params = navzorkuj_parametry(tape, m, out.len())  // DIR/ADDR/CONST/HERE
        slots  = expand(m, params)                         // fragment → Vec<u32>
        připoj slots do out, ořízni poslední makro na `energy`  // částečný plátek = OK (R2)
    return out
```

- **Determinismus:** čistá funkce `(seed, energy)`. Stejný seed → stejná paměť.
- **Konzervace:** generátor jen plní *počáteční* paměť; `energy == mem_len`
  triviálně platí (stejně jako u dnešního šumu). Žádný runtime invariant se
  nedotýká.
- **Bezpečnost vůči VM:** každé makro expanduje na platné instrukce; pod fold
  dekodérem je navíc *každý* byte platný, takže i oříznutý poslední fragment je
  bezpečný.
- **Vzorkování adres:** modulární adresace dělá libovolný u32 bezpečným; volitelně
  biasovat `ADDR` k nízkým adresám (stabilní jádro), `HERE` = aktuální offset.

### Zasazení do `big_bang`

Dnešní API má tři režimy fillu schované za dvěma funkcemi. Návrh je sjednotit do
**explicitního režimu genesis**:

```rust
enum Genesis<'a> {
    Noise,             // dnešní RNG fill — zůstává pro baseline/testy
    Program(&'a [u32]),// autorský prefix + šumový ocas (dnešní big_bang_with_program)
    Macros,            // NOVÉ: aperiodický stream maker přes celou paměť
}
// big_bang(seed, energy) == Genesis::Noise; big_bang_with_program == Genesis::Program
```

- **`Macros` se stává výchozím režimem světa** (frontend Reset, WASM `World.new`),
  protože nahrazuje umělý šum bohatším startem (`gravity-plan.md`). `Noise`
  **zůstává** pro baseline a šumové invariantní testy (analogie: `step_diffusion`
  vedle `step`).
- **Hráčův program** (exogenní vstup, `gravity-plan.md` novost #2) má přednost:
  `Program(prefix)` přepíše začátek; otevřená otázka, zda zbytek dofiltrovat
  šumem (dnes) nebo makry (`Program` + `Macros` ocas) — viz „Otevřené otázky".

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
- **Interakce s hráčovým programem.** Prefix `Program` + co dál: šumový ocas
  (dnes) vs makro ocas? Makro ocas je konzistentnější s R2, ale mění sémantiku
  dnešního `big_bang_with_program`.
- **Vzorkování `ADDR`.** Uniformně (modulární, bezpečné) vs bias k nízkým adresám
  (stabilní jádro). Ovlivní, jak často makra píšou do „programové" vs „datové"
  zóny.
- **Self-reference (`HERE`).** Stačí offset začátku makra, nebo je užitečný i
  odkaz na globální start programu (silnější replikátor)?
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

## Návaznost

Genesis **staví na** `opcodes-plan.md` (hustá sada + fold — bez nich nejsou makra
expresivní a fragmenty nejsou platné) a **krmí** `gravity-plan.md` (seedovaný
informaci-nesoucí start je palivo pro selekci v gravitačních dolech a substrát pro
mutaci). Předpoklad „hustá sada + fold" je **splněn** (commit `f252784`), takže
v1 makro-genesis může jít hned. Gravitace/mutace se dodává paralelně; až bude
v jádře, genesis jí dodá informaci-nesoucí start místo šumu — pořadí mezi nimi
není kritické, jsou nezávislé až po bod, kde se mutace případně naváže na
seedovou pásku (R1, otevřená otázka).
