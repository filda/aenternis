# Prototyp 5: pointery, reprodukce a infekce

Pátý laboratorní prototyp ověřuje pasivní vyzařování řízené šesti směrovými pointery jako jediný mechanismus reprodukce, infekce a mutace. Každá buňka v 3D toroidálním poli je latentní mikro-entita s vlastní pamětí.

**Slot model**: paměť buňky je posloupnost 32-bit unsigned integer slotů. Každý slot může obsahovat libovolnou hodnotu 0 až 2^32-1. Opcode instrukce je nejnižší bajt slotu (`slot & 0xFF`), zbytek slotu je k dispozici jako embedded immediate hodnota nebo se ignoruje. Operandy instrukcí jsou celé sloty, takže instrukce `set <addr>, <value>` zabírá vždy 3 sloty bez ohledu na velikost adresy nebo hodnoty - velké cely (megabajty paměti) jsou plně adresovatelné stejným formátem instrukcí.

## Spuštění

Otevři `index.html` v prohlížeči. Není potřeba build, npm ani lokální server.

## Etapa 1: pole mikro-entit a difuze jako míchání

Aktuální stav prototypu implementuje první etapu z `PROTOTYP-05-PLAN.md`:

- 3D toroidální grid (default 32³)
- každá buňka má energii (= velikost paměti v bajtech) a paměťový obsah
- systém v každém taktu spočítá rate vyzařování v každém směru z každé buňky podle rozdílu potenciálů se sousedem
- systém rozloží pointery od konce paměti dolů podle těchto rate (`zn_ptr = mem_size - rate_zn`, ... `xp_ptr = mem_size - sum(rates)`)
- odtok: pro každou buňku a každý směr se zkopíruje rate_d bajtů počínaje pointerem ven
- přítok: bajty od šesti sousedů se přiloží na konec paměti
- konec taktu: systém přepočítá rate a layout pro další takt

CPU vykonávání instrukcí, programátorský override pointerů a aktivní zápis na port přijdou v dalších etapách.

## Otázka etapy 1

Vzniká pozorovatelně dvouvrstva (stabilní jádro + míchající membrána) z asymetrických počátečních podmínek?

## Co sledovat

- **Heatmapa energie**: chování difuze. Při scénáři "Hustá koule" by měla energie postupně difundovat ven, jako v prototypu 1.
- **Inspector libovolné buňky**: kliknutím do canvasu se vybere buňka v aktuálním řezu. V panelu inspectoru je vidět:
  - aktuální energie a velikost paměti
  - hodnoty šesti pointerů (xp, xn, yp, yn, zp, zn)
  - rate v každém směru (= odečet sousedních pointerů)
  - hex dump paměti se zvýrazněnými pozicemi pointerů
- **Vizualizace "vrch paměti" vs "spodek paměti"**: porovnání obsahu na vysokých a nízkých adresách. Pokud teorie funguje, spodek paměti by měl být dlouhodobě stabilnější (jádro), zatímco vrch by se měl měnit (membrána).
- **Scénář "Dvě koule s odlišným obsahem"**: pravá koule má 0x55, levá 0xAA. Postupně by se mělo rozhraní rozmazat - difuze nese obsah s sebou.

## Parametry

- **Velikost (N×N×N)**: rozměr toroidálního světa
- **Počáteční scénář**: počáteční rozložení energie a obsahu
- **Počáteční energie buňky (max)**: maximum energie v jakékoliv buňce na začátku
- **Koeficient toku**: rate per Δpotenciál. Hodnota je omezená < 1/6, jinak vznikají numerické nestability (víc se vyzáří, než je v paměti).

## Známé limity

- Tento prototyp ještě neimplementuje CPU. Buňky jen difundují energii a obsah, neexekvují program.
- Při velmi velkém světě (>32³) se tikání zpomalí kvůli vytváření nových Uint8Array per buňku.
- Pořadí příchozích bajtů od různých sousedů je pevné (xp, xn, yp, yn, zp, zn).
- Nad koeficient toku 0.17 se objevuje šachovnicový artefakt, vlastní explicitní 3D difuzi nad hranicí stability. Energie se přitom dál zachovává - jen rozložení je zubaté místo plynulého.

## Stav etap

- **Etapa 1**: hotová. Difuze jako míchání produkuje očekávanou dvouvrstvu (stabilní jádro + měnící se membrána). Stochastický floor řeší zamrzání při nízkých gradientech, proporční clampování řeší šachovnicové artefakty při vysokém koeficientu.
- **Etapa 2 + 3**: hotové. Každá buňka vykonává `energie / K` instrukcí za takt světa (akumulátor `compute_credit` pro zlomky), inspector zobrazuje PC, poslední instrukci a celkový počet provedených instrukcí. V inspectoru lze přes textarea injektovat program do vybrané buňky.

### Instrukční sada

| opcode | mnemonic  | délka  | význam |
|--------|-----------|--------|--------|
| `0x00` | `nop`     | 1 slot | nedělá nic |
| `0x01` | `set a v` | 3 sloty | mem[a] = v |
| `0x02` | `copy a b`| 3 sloty | mem[a] = mem[b] |
| `0x03` | `add a b` | 3 sloty | mem[a] += mem[b] (mod 2^32) |
| `0x04` | `sub a b` | 3 sloty | mem[a] -= mem[b] (mod 2^32) |
| `0x05` | `inc a`   | 2 sloty | mem[a] += 1 (mod 2^32) |
| `0x06` | `dec a`   | 2 sloty | mem[a] -= 1 (mod 2^32) |
| `0x07` | `jmp a`   | 2 sloty | PC = a (mod memSize) |
| `0x08` | `jz a t`  | 3 sloty | if mem[a] == 0: PC = t |
| `0x09` | `setp d v`| 3 sloty | pointers[d % 6] = v (ephemeral, do konce taktu) |
| `0x0A` | `getp d a`| 3 sloty | mem[a] = pointers[d % 6] |
| `0x0B` | `port d i`| 3 sloty | active outflow: posílá `i` slotů ve směru `d % 6` nad rámec passive rate (projektil) |

Opcode = `slot & 0xFF` (nejnižší bajt slotu). Adresování modulo aktuální velikosti paměti. Žádná ochrana paměti. Aritmetika přetéká přes 2^32. Neznámý opcode (>0x0A) se chová jako `nop` (krok o 1 slot).

Hustota smysluplných opcodů: aktuálně 11/256 = 4.3%. Hustota se v pozdější etapě rozšíří přidáním více instrukcí (and/or/xor/shl/shr, jnz/je/jl, push/pop, atd.) blíže k Z80 stylu (~60%+).

### Předvolby programů (dropdown v inspektoru)

- **Counter**: nejmenší živý program. `inc 0x10; jmp 0`. Buňka stárne v paměťové buňce 0x10. Sám sebe nereplikuje.
- **Self-replicator → +x**: `setp xp, 0x00; jmp 0`. Každý takt přesměruje výchozí xp_ptr na začátek vlastního programu, takže výchozí radiace v +x nese program. Sleduj inspector souseda v +x.
- **Self-replicator → všech 6 směrů**: 6× `setp d, 0x00`, pak `jmp 0`. Broadcast do všech směrů.
- **Beacon**: `inc 0x20; setp xp, 0x00; jmp 0`. Inkrementuje počítadlo + vyzařuje program s narůstající hodnotou.
- **Quine s chráněným jádrem**: program, který v paměti opakovaně obnovuje DEADBEEF na adresách 0x10-0x13 a vyzařuje ho. Demonstruje, jak držet payload proti náhodnému přepisu z difuze.
- **DEADBEEF (data)**: čtyři bajty, žádný program. Nejjednodušší způsob, jak vidět difuzi obsahu.
- **Projektil → +x**: každý takt přesměruje xp_ptr na vlastní program a triggeruje silný active write 32 slotů ve směru +x. Daleko silnější dosah než pasivní self-replicator. Naopak rychleji vyčerpá zdrojovou energii (vyzařuje víc než difuze sama).

### Vizualizace

- **Energie**: heatmapa energetického pole (jako prototyp 1).
- **Vrch / Spodek paměti**: barva podle hodnoty bajtu na nejvyšší / nejnižší adrese. Ukáže, jak se membrána odlišuje od jádra.
- **Aktivita CPU**: zvýrazní buňky, které vykonaly nenulový počet smysluplných instrukcí v posledních ~20 taktech (exponential decay 0.95). V poli šumu se rozsvítí ručně vložené programy nebo náhodně vznikající smyčky.
