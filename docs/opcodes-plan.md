# Aenternis — plán: zhuštění instrukční sady (opcode density)

Last updated: 2026-06-13 (✅ IMPLEMENTOVÁNO — viz „Stav implementace" na konci)

Designový kontext žije v `vm.md` (spec instrukční sady) a `aenternis.md`;
obecný roadmap v `plan.md`; gravitační návra v `gravity-plan.md` (ten na hustší
sadu explicitně čeká: „Zhušťování instrukční sady je páka, ne parametr").

## Otázka, kterou plán odpovídá

Opkód = `slot & 0xFF`, takže existuje 256 hodnot. Dnes je definováno 20 opkódů
(`0x00`–`0x13`), zbytek dekóduje jako `nop`. Z náhodného třeskového šumu tedy
**~92 % bytů nedělá nic** → svět z šumu skoro nepočítá a emergence vázne. Cíl
z `plan.md`: ~60 % smysluplných opkódů („Z80 level").

Jádro rozhodnutí: **hustotu neřeší počet opkódů, řeší ji dekodér.** Když se dekód
změní na `op = (slot & 0xFF) % N`, je 100 % bytů platná instrukce *bez ohledu na
to, kolik jich definujeme*. Tím se „60 %" stává automaticky splněným a odpadá
tlak přidávat opkódy jen kvůli číslu. Instrukce pak přidáváme **podle skutečné
potřeby**, ne podle kvóty.

Plán proto má dvě fáze, vědomě v tomto pořadí:

1. **Fáze 1 — přidat instrukce, o kterých už víme, že je budeme chtít.** Čistě
   aditivní, zpětně kompatibilní, žádná změna dekodéru → nízké riziko.
2. **Fáze 2 — fold dekodéru** (`% N`). Mění sémantiku „neznámý → nop" a vyžaduje
   re-bless šumových testů → vyšší riziko, ale dělá hustotu navždy vyřešenou.

## Invarianty, které se nesmí porušit

Každý nový opkód i fold musí ctít:

- **Konzervace energie = `energy == mem_len`.** Drtivá většina navržených
  instrukcí mění jen `mem[a]` *na místě* (nemění délku paměti) → konzervace je
  triviálně zachována, stejně jako u stávajících `add`/`sub`/`inc`/`dec`. Jediné
  strukturální výjimky (stack) jsou vědomě odloženy (viz „Mimo rozsah").
- **Determinismus.** Žádný opkód ani fold nesmí záviset na pořadí iterace ani
  zavádět nový zdroj náhody. Fold je čistá funkce bytu. Zamrzlá off-by-one RNG
  konvence (layout pro tick N počítán na konci N−1) se nemění.
- **Introspekční invariant** (`vm.md`): žádná nová instrukce nesmí číst vnitřek
  sousední buňky. Navržené instrukce jsou všechny čistě lokální (mem ↔ mem),
  takže invariantu se ani nedotýkají.
- **Bezpečnost VM proti šumu.** Žádný vstup nesmí shodit VM (panika). Konkrétně:
  `shl`/`shr` maskují počet posunů `& 31` (Rust posun u32 o ≥32 panikuje);
  `div`/`mod` mají definované chování při nulovém děliteli (viz níže).

## Fáze 1 — instrukce podle potřeby

Výchozí návrh sady. **Je to návrh k ořezání**, ne pevný seznam — řídí se
„nepřidávat bezhlavě". Vychází z „Planned extensions" v `vm.md` (aritmetika,
bitové operace, znaménkové skoky). Senzory (`sinflow`/`sself`/`srate`) sem
**nepatří** — byly z plánu vědomě vyškrtnuty (commit `eee42b7`).

Navržené opkódy, navazují od `0x14` (uvolněno po vyškrtnutí senzorů):

| Opcode | Mnemonic   | Délka | Sémantika | Konzervace |
|--------|------------|-------|-----------|------------|
| `0x14` | `and a b`  | 3 | `mem[a] &= mem[b]` | in-place ✓ |
| `0x15` | `or a b`   | 3 | `mem[a] \|= mem[b]` | in-place ✓ |
| `0x16` | `xor a b`  | 3 | `mem[a] ^= mem[b]` | in-place ✓ |
| `0x17` | `not a`    | 2 | `mem[a] = !mem[a]` (bitový doplněk) | in-place ✓ |
| `0x18` | `shl a b`  | 3 | `mem[a] <<= (mem[b] & 31)` | in-place ✓ |
| `0x19` | `shr a b`  | 3 | `mem[a] >>= (mem[b] & 31)` (logický) | in-place ✓ |
| `0x1A` | `mul a b`  | 3 | `mem[a] = mem[a].wrapping_mul(mem[b])` | in-place ✓ |
| `0x1B` | `div a b`  | 3 | `mem[a] = mem[b]==0 ? 0 : mem[a] / mem[b]` (unsigned) | in-place ✓ |
| `0x1C` | `mod a b`  | 3 | `mem[a] = mem[b]==0 ? 0 : mem[a] % mem[b]` (unsigned) | in-place ✓ |
| `0x1D` | `jp a t`   | 3 | znaménkově: `if (i32)mem[a] > 0 { PC = t }` | jen PC ✓ |
| `0x1E` | `jn a t`   | 3 | znaménkově: `if (i32)mem[a] < 0 { PC = t }` | jen PC ✓ |

→ 31 opkódů (`0x00`–`0x1E`).

**Proč právě tyhle:**

- **Bitové (`and`…`shr`)** mají nejvyšší hodnotu pro emergenci: dovolí programu
  *syntetizovat* hodnoty, adresy a samotné opkódy z bitů (maskování, skládání).
  To je přesně to, co umožňuje, aby z náhodného šumu vznikl smysluplný kód.
- **`mul`/`div`/`mod`** doplňují aritmetiku (dnes jen add/sub/inc/dec).
- **`jp`/`jn`** přidávají znaménkové větvení (dnes jen `jz`/`jne` proti nule).
  Levné (3 sloty, test proti nule), dobře se páruje se `sub`. Dvouoperandové
  znaménkové porovnání (`jlt a b t`, `jgt a b t`) vědomě odloženo — `sub` + `jn`
  ho zastoupí, a 4slotové instrukce jsou v hustém VM dražší.

**Rozhodnuto — `div`/`mod` nulou → výsledek 0** (`checked_div` /
`checked_rem(…).unwrap_or(0)`). Reálné HW by trapovalo (x86 `#DE`), ale
trap/přerušení nemáme a zatím nepotřebujeme; výsledek 0 je deterministický,
uniformní (vždy zapíše cíl, bez větvení v sémantice) a triviálně testovatelný.
Posun u `shl`/`shr` je `mem[b]` maskovaný `& 31` (ne immediate), kvůli konzistenci
s `add`/`sub` a bezpečnosti (Rust posun u32 o ≥32 panikuje).

### Synchronizační body (Fáze 1)

Přidání jednoho opkódu = úprava na těchto místech (vše musí zůstat v souladu,
jinak se rozjede PC nebo disassembler):

1. **`crates/aenternis-core/src/vm.rs`**
   - `Opcode` enum: nová varianta s diskriminantem = byte.
   - `decode`: nová větev `0x14 => Some(Self::And)` …
   - `length`: zařadit do správné délkové skupiny.
   - `ALL`, `COUNT` (20 → 31), `MAX` (`0x13` → `0x1E`).
   - `execute_instruction`: nová `match` větev se sémantikou.
2. **`src/asm.ts`** — `OPCODES` mapa (mnemonic → `{code, args}`).
3. **`src/disasm.ts`** — dědí z `OPCODES` automaticky; žádný z nových opkódů
   nemá směrový operand, takže `DIR_ARG_OPS` se **nemění**. Ověřit formátování.
4. **`docs/vm.md`** — řádky v tabulce instrukcí + aktualizovat „Opcode density".
5. **Testy:**
   - `tests/vm_opcode.rs` — decode/length/ALL/COUNT/MAX páry (rozšířit pole).
   - `tests/vm_execute.rs` — per-opkód chování (1 test na opkód: typický případ
     + hrana: `div`/`mod` nulou, `shl`/`shr` o ≥32, znaménkové hrany `jp`/`jn`).
   - `tests/asm.test.ts` — počet `OPCODES` (20 → 31), round-trip asm.
   - `tests/disasm.test.ts` — render nových mnemonik.
   - Konzervace/determinismus (`tests/tick_step.rs` ap.) musí zůstat zelené beze
     změny — nové opkódy jsou konzervaci-bezpečné.

## Fáze 2 — fold dekodéru

Změna jádra dekódu tak, aby **každý** byte mapoval na platnou instrukci:

```rust
// dnes: decode(slot) -> Option<Opcode>, byte > MAX => None => nop
// nově: decode je totální
pub const fn decode(slot: u32) -> Opcode {
    let folded = (slot as u8) % Opcode::COUNT;   // 0..COUNT-1
    // folded vždy odpovídá platnému diskriminantu, protože opkódy
    // obsazují souvislé 0..COUNT-1
    …
}
```

**Klíčová vlastnost:** diskriminanty opkódů musí být souvislé `0..COUNT-1` (dnes
jsou — `0x00`–`0x13` bez děr; Fáze 1 to udrží). Pak je `% COUNT` vždy platný a
fold se sám přizpůsobí jakémukoliv počtu opkódů. **Hustota = 100 %, navždy, bez
ohledu na to, kolik opkódů přidáme později.**

Zpětná kompatibilita: byty `0..COUNT-1` se foldem nemění (`b % COUNT == b`), takže
**všechny existující sestavené programy a presety běží identicky**. Mění se jen
chování bytů `≥ COUNT` (dříve nop, nově alias na `b % COUNT`).

### Kompatibilita při *dalším* přidávání opkódů

Častá obava: „nerozbije fold programy, až později přidám reálný opkód a `COUNT`
poroste?" **Autorské (sestavené) programy ne.** Záruka stojí na dvou pravidlech:

1. Opkódy obsazují **souvislé** `0..COUNT-1`.
2. Nové opkódy se vždy **jen připojují na konec** (další volný diskriminant) —
   nikdy se nepřečíslovávají ani nevkládají doprostřed. `and` zůstane `0x14`
   navždy.

Sestavený program používá byty = diskriminanty opkódů, tedy `< COUNT_staré`. Po
přidání opkódů je `COUNT_nové > COUNT_staré`, takže každý ten byte je stále
`< COUNT_nové` ⇒ `b % COUNT_nové == b` ⇒ **dekóduje na týž opkód.** Přidání množinu
jen rozšiřuje, existující čísla nemění.

**Co se posune:** jen slot, jehož **nízký byte je `≥ COUNT` a vykoná se jako
instrukce** (mění se modulus: `200 % 31 = 14`, ale `200 % 40 = 0`). To se týká
(a) RNG třeskového šumu a (b) exotického self-modifying kódu skákajícího do
datových slotů. U (a) jde o fakt, že nová verze ISA rozvíjí seed jinak — **replay
napříč verzemi ISA nebyl nikdy slíben** (viz `plan.md`, 2026-05-13; replay platí
uvnitř verze). U (b) jde o už tak křehký, neobvyklý vzor.

Pozn.: tohle není slabina foldu — i dnešní režim „neznámý → nop" mění význam
vysokých bytů při přidání opkódu (nop → reálná instrukce). Fold jich posune víc
najednou (mění se modulus, ne jen jedna hodnota), ale **třída dotčených programů
je stejná úzká**: byty `≥ COUNT` vykonávané jako kód. **Maintenance pravidlo:**
opkódy přidávat výhradně append-only, nikdy nepřečíslovat — jinak se záruka
poruší.

### Důsledky a co re-blessnout

- **`vm.rs`**: `decode` vrací `Opcode` místo `Option<Opcode>`; zmizí větev
  „unknown → nop, advance by 1" v `execute_instruction` (nop je nadále dosažitelný
  pro byty `0, COUNT, 2·COUNT…`). `MAX` ztrácí smysl jako „hranice platnosti" —
  buď zrušit, nebo předefinovat na `COUNT-1`.
- **`tests/vm_opcode.rs`**: `decode_returns_none_for_undefined_bytes` a část
  `decode_ignores_upper_24_bits` (větev s `None`) se musí přepsat na ověření
  foldu (`decode(byte) == ALL[(byte % COUNT)]`).
- **`src/disasm.ts`**: aby disassembler věrně ukazoval, co VM *opravdu* vykoná,
  musí fold zrcadlit — byte `≥ N` zobrazit jako jeho foldnutou mnemoniku
  (volitelně s poznámkou, že šlo o aliasovaný byte). Cesta „raw" zůstává jen pro
  případ, kdy by instrukce přetekla konec slotového pole.
- **`vm.md`**: přepsat sekce „Slot and opcode" (zrušit „unknown = nop") a „Opcode
  density" (hustota je nově strukturálně 100 %).
- **Šumové stavové testy**: testy, které pouštějí VM nad šumem a zmrazují
  *konkrétní* výsledný stav, se změní (šum teď aktivně počítá místo nopování) →
  re-bless. **Invariantní testy** (konzervace, determinismus, `world.len() ≤
  E_total`, bit-parita rayon cesty) musí projít **beze změny** — fold je
  deterministický a nové chování je konzervaci-bezpečné. Pokud invariantní test
  spadne, je to skutečný bug, ne re-bless.

### Otevřené otázky k foldu

- **Nerovnoměrnost mapování.** `256 = k·N + r`; prvních `r` opkódů dostane o
  jeden byte navíc (mírná převaha nízkých opkódů z čistého šumu). Při `N=31`:
  `256 = 8·31 + 8` → opkódy `0x00`–`0x07` mají váhu 9, zbytek 8. Zanedbatelné, ale
  zdokumentovat. (Chceme-li dokonale uniformní, šlo by řadit „nejčastější/nejmíň
  rušivé" opkódy na nízké pozice — `nop` je `0x00`, což je rozumné.)
- **Pořadí opkódů jako ladicí páka.** Protože nízké diskriminanty jsou ze šumu
  o chlup pravděpodobnější, pořadí v enumu mírně ovlivňuje statistiku náhodných
  programů. Zatím neřešit; flag pro pozdější ladění emergence.

## Postup a brány

Každá fáze končí na zeleném `./check` (rustfmt + clippy -D warnings + cargo test
+ vitest/coverage + WASM build) **a** `cargo mutants` bez nových mezer (mutační
testování je součást brány kvality pro algoritmické změny).

**Pre-public zjednodušení:** aplikace zatím není veřejná, takže kompatibilita
není tvrdé omezení a **obě fáze lze dodat jako jeden krok** — fold dekodér se
napíše rovnou s novými opkódy (žádné psaní `Option`-vracejících větví, které by
se hned mazaly), re-bless šumových testů proběhne jednou. Append-only + souvislé
`0..COUNT-1` se drží dál, protože to fold potřebuje ke korektnosti.

Doporučený interní postup (i v rámci jednoho kroku):

- nejdřív přidat opkódy (bitové → aritmetika → znaménkové skoky) a ověřit zeleno
  — čistě aditivní, žádný re-bless;
- pak fold dekodéru (totální `decode` + zrcadlení v `disasm` + re-bless šumových
  stavových testů + dokumentace), opět zeleno.

## Mimo rozsah (zatím)

- **Stack (`push`/`pop`/`call`/`ret`).** Jediná plánovaná rodina, co potřebuje
  nový registr `SP` a má nedořešené strukturální otázky: kde stack v paměti žije,
  kolik má slotů, jak se chová při shrinku paměti (outflow zmenšuje `mem_len`) a
  při přetečení/podtečení. Dotýká se `Cell` stavu i konzervačního invariantu →
  samostatný follow-up s vlastním návrhem, ne součást tohoto kroku.
- **Rotace (`rol`/`ror`), `neg`, dvouoperandové znaménkové skoky (`jlt`/`jgt`).**
  Nízká marginální hodnota nad navrženou sadou; přidat on-demand, fold je beze
  změny pojme.
- **Multi-hop senzory.** Otevřená otázka z `vm.md`, nesouvisí s hustotou.

## Návaznost

Po Fázi 2 je hustota strukturálně vyřešená a `gravity-plan.md` má splněný svůj
předpoklad („hustší instrukční sada") — gravitaci/tlak lze napojit do `tick.rs`
a zkoumat emergenci tam, kde už žije VM a `active_outflow`.

## Stav implementace (2026-06-13)

✅ **Hotovo, brána zelená** (`./check` ALL GREEN + `cargo mutants` na `vm.rs`:
154 mutantů, 153 caught, 1 unviable, **0 missed**).

- **Fáze 1** — 11 opkódů `0x14`–`0x1E` (`and`, `or`, `xor`, `not`, `shl`, `shr`,
  `mul`, `div`, `mod`, `jp`, `jn`) přidáno do `vm.rs` (enum, `length`, `ALL`,
  `COUNT=31`, `MAX=0x1E`, execute větve), `src/asm.ts` (`OPCODES`) a `docs/vm.md`.
  `div`/`mod` nulou → 0; `shl`/`shr` přes `wrapping_shl`/`wrapping_shr`
  (posun `mod 32`); `jp`/`jn` znaménkově bez `as i32` castu (MSRV 1.85 předchází
  `u32::cast_signed`) — test sign-bitu.
- **Fáze 2** — `Opcode::decode` je totální: `ALL[(slot as u8) % COUNT]`. Zmizela
  větev „unknown → nop" v `execute_instruction`. `src/disasm.ts` zrcadlí fold
  (`(slot & 0xFF) % OPCODE_COUNT`); „raw" zůstává jen pro uříznutý konec dumpu.
- **Re-bless** — baseline state-hashe v `tests/apply_outflow_bit_parity.rs`
  přegenerovány **dvakrát** (jednou po Fázi 1, jednou po foldu), protože šumové
  byty teď vykonávají reálné instrukce. Konzervace, determinismus a world-size
  invarianty prošly beze změny → drift je legitimní, ne skrytý bug.
- **Append-only invariant** zafixován dokumentačně v `vm.rs` i `asm.ts`: opkódy
  musí zůstat souvislé `0..COUNT-1` a jen se připojovat na konec, jinak fold
  rozbije existující programy.

**Odloženo (beze změny plánu):** `stack` (push/pop/call/ret — potřebuje `SP`),
`neg`/`rol`/`ror`, dvouoperandové znaménkové `jg`/`jl`. Fold je všechny pojme
beze změny dekodéru, až je přidáme.
