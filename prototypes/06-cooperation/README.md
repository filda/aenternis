# Prototyp 6: kooperace dvou entit

Šestý laboratorní prototyp se zaměřuje na otázku, **může-li mezi dvěma sousedními entitami vzniknout stabilní spolupráce čistě skrze program a porty**, bez toho, aby engine znal pojem "spojenec".

Prototyp je 2D varianta prototypu 5. Zachovává:

- slot model (32-bit unsigned integery jako paměťové sloty)
- VM s 20 opcody (nop, set, copy, add, sub, inc, dec, jmp, jz, setp, getp, port, senergy, jne, je, ldi, sti, setpv, sid, paint)
- pasivní vyzařování s informačním obsahem
- aktivní zápis na port (`port d, i`) jako silný outflow
- layout pointerů od konce paměti
- difuze jako míchání s membránou a stabilním jádrem

Změny oproti prototypu 5:

- 2D toroidální pole (4 sousedi: xp, xn, yp, yn) místo 3D
- žádný řez/osa - canvas vykresluje přímo celý svět
- **dva inspectory vedle sebe** pro cely A a B
- **komunikační stopa** A ⇄ B - pokud sousedí, ukazuje, jaké sloty si poslaly v posledním taktu
- `port` instrukce bere směr 0-3 místo 0-5

## Spuštění

Otevři `index.html` v prohlížeči. Bez build kroku.

## Otázka prototypu

Lze napsat program, který v dvojici sousedních cel:

- udržuje stabilní vzájemný kontakt přes desítky až stovky tiků
- vzdoruje rozkladu okolním šumem nebo agresivními sousedy
- využívá "vnitřní stěnu" (společný okraj A+B) jako fyzikální výhodu vůči izolované cele

A vznikne taková spolupráce **jen z programu**, ne ze speciálního engine pravidla?

## Workflow

1. Reset s počátečním scénářem (typicky "Rovnoměrný šum" pro pozadí, "Tiché pozadí" pro klidnější laboratoř, nebo "Prázdné pole" pro čistý experiment)
2. Klikni do canvasu - vybírá se cela B. Shift+klik vybírá A.
3. Pro každou ze dvou cel: vyber preset z dropdownu nebo napiš hex bajty, klikni "Vlož".
4. Spusť simulaci a sleduj, jak se A a B vzájemně ovlivňují.
5. **Komunikační stopa** v pravém panelu ukazuje, co A poslalo B a B poslalo A v posledním taktu.

## Náměty na experimenty

- **Mutual reinforcement**: do A i B vlož stejný `self_xp`. Pokud A je vlevo od B, A vyzařuje program v +x do B, ale B vyzařuje v +x dál pryč od A. Není symetrické.
- **Mirror pair**: do A vlož `setp xp, 0; jmp 0` (broadcast +x), do B vlož `setp xn, 0; jmp 0` (broadcast -x). Pokud B sedí v +x od A, oba vyzařují **navzájem na sebe**. Mělo by vzniknout něco jako rezonance.
- **Quine duo**: oba s `quine_core`. Pokud sousedí, mohou si jádra navzájem reinforcovat?
- **Defender vs invader**: jeden je projektil, druhý je quine. Kdo přežije déle?

## Nové instrukce v prototypu 6

Pro experimenty s kooperací byly přidány instrukce nad rámec prototypu 5:

| Opcode | Mnemonic     | Délka  | Sémantika |
|--------|--------------|--------|-----------|
| `0x0C` | `senergy d a`| 3 sloty | `mem[a] = energie souseda ve směru d` (read-only sense) |
| `0x0D` | `jne a t`    | 3 sloty | skok pokud `mem[a] != 0` |
| `0x0E` | `je a b t`   | 4 sloty | skok pokud `mem[a] == mem[b]` |
| `0x0F` | `ldi a b`    | 3 sloty | `mem[a] = mem[mem[b]]` - load indirect |
| `0x10` | `sti a b`    | 3 sloty | `mem[mem[a]] = mem[b]` - store indirect |
| `0x11` | `setpv d a`  | 3 sloty | `pointers[d % DIRS] = mem[a]` - setp s runtime hodnotou |

Senzory pracují jen na vzdálenost 1 (bezprostřední soused). Pro detekci aktivity pošleme `senergy` na soused → uložíme do paměti → porovnáme se starou hodnotou (`je`/`jne`).

`ldi`/`sti`/`setpv` umožňují **indirect addressing** - cílová adresa je dynamicky čtená z paměti, ne zafixovaná v instrukci. Klíčové pro programy, které chtějí zpracovat hodnoty, jejichž místo se rozhodlo až za běhu.

## Asembler (2026-05-01)

Inject textarea zvládá oba formáty:

**Raw hex** (zpětná kompatibilita):
```
05 04 07 00
DE AD BE EF
```

**Asembler** s mnemoniky, labely a komentáři:
```
; Counter program
loop:
  inc 0x10
  jmp loop
```

**Self-replicator přes label:**
```
start:
  setp xp, start
  jmp start
```

**Mix programu a dat:**
```
loop:
  setp xp, data
  jmp loop
data:
  DE AD BE EF
```

**Direction names**: `xp`=0, `xn`=1, `yp`=2, `yn`=3 (lze použít místo čísel u setp/getp/port/setpv/senergy)

**Komentáře**: středník `;` na konci řádku

**Kooperační program s podmínkou:**
```
start:
  senergy xp, 0x20      ; energie souseda v +x
  senergy xn, 0x21      ; energie souseda v -x
  je 0x20, 0x21, balanced
  jmp start
balanced:
  setp xp, start
  port xp, 16
  jmp start
```

Asembler je dvouprůchodový - labely lze použít forward i backward. Mnemoniky jsou case-insensitive. Hex literály bez prefixu (DE AD BE EF) i s prefixem (0xDE) jsou podporovány.

## Mechanika dominance + intrusion (2026-05-01)

Phase 2 step() implementuje měkké promíchání kontinuit při kolizi. Pro každý inflow směr:

```
attacker_E_post_burn = neighbor_E - sum_neighbor_combined_rates
r = target_E / max(attacker_E_post_burn, 1)
dominance = clamp(1 - r / move_threshold, 0, 1)
intrusion_depth = floor(dominance * current_memSize)
write_start = max(0, current_memSize - intrusion_depth)
```

Inflows se setřídí podle **dominance descending** a vsouvají do paměti od nejsilnějšího (do nejnižších adres) k nejslabšímu (k povrchu). Vsuvka (insert) - existující obsah se posune nahoru, energie/memSize roste o velikost inflow.

Důsledky podle dominance:

- **dominance ≈ 0** (cíl silnější): inflow vsunut na konec = stará "append" sémantika, povrchový dotek
- **dominance ≈ 0.5** (vyrovnaný souboj): inflow uprostřed paměti, "zaseklá kontinuita"
- **dominance ≈ 1** (drcivá převaha): inflow zaujme nejnižší adresy, **plná metempsychóza**. PC zůstává numericky stejné, takže pokud byl pc_old < write_start, program pokračuje (jádro chráněné). Pokud pc_old >= write_start, PC ukazuje teď na útočníkovu paměť → exekuce přejde na útočníkův kód.

`move_threshold` lze ladit slidrem (default 2.0). Vyšší threshold = větší dominance při stejných energiích.

## Známá omezení

- communication trace ukazuje jen poslední takt, žádná historie/graf
- senzory vidí jen bezprostředního souseda, ne dál
- nelze přímo přečíst skutečný obsah inboxu (programátor musí najít místo v paměti přes pointery)
- chybí senzor vlastního stavu (vlastní energie, memSize) - tyto se musí odvozovat
- *(implementováno 2026-05-02)* origin tag a war paint jsou teď součástí prototypu

## Identita: tag, paint, lineage tracker (2026-05-02)

Každá buňka má dva 32-bit metadata fieldy mimo herní fyziku:

- **originTag**: unikátně náhodně přidělená identita při resetu. Lze přečíst přes opcode `sid a` - call-sign pro program.
- **appearance**: war paint, default 0. Lze nastavit přes opcode `paint v` - cela si volí svůj vzhled.

Žádné z těchto polí neovlivňuje rate, dominance, ani další fyzikální výpočty.

### Vizualizace

Dva nové režimy v "Vizualizace":

- **War paint**: HSV mapping. Hue z appearance (programátorská volba), brightness z energie (auto-scale). Cely s appearance = 0 jsou šedé. Programy můžou používat `paint` pro signalizaci stavu (alert, idle, active...).
- **Identity (origin tag)**: stejné HSV mapping, ale hue z origin tagu. Při resetu jsou všechny barvy unikátní; jak se cela "stane jinou entitou" metempsychózou (paměť přepsaná dominantním útokem), origin tag se zatím **nemění** (cela má pořád svůj náhodný tag z resetu, jen s cizím obsahem). Future: dominance-propagation tagu.

### Lineage tracker

Tlačítko "Track" u každého inspectoru:

- Klik 1: zachytí snapshot prvních 16 slotů cely. Tlačítko se přebarví (Stop track).
- Při běhu simulace: každý takt projde celý svět, najde celu s nejlepší shodou s snapshot na nízkých adresách (počet identických slotů). Inspector se přesune na tu celu.
- Klik 2: zastaví trackování.

Tím vidíš, kam se "tvoje" entita posunula skrze metempsychózu. Když dominance přesune kontinuitu programu do souseda, jeho jádrová paměť teď obsahuje útočníkův program → nejlepší match je tam.

Pokud se kontinuita rozpadne (silná infekce všemi směry, žádná cela neuchovává původní vzor), tracker uvízne na nejbližší cele - lineage je ztracená.
