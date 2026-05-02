# Prototyp 7: 3D performance test

Sedmý laboratorní prototyp **neuvažuje nové fyzikální mechaniky**. Jeho úloha je čistě praktická: změřit, jak daleko se v JavaScriptu dostaneme s 3D simulací Aenternis pole.

Klíčové otázky:

- jakou velikost N (3D mřížka N×N×N) JS zvládne plynule
- kolik je ms / tik světa při různých N
- kolik FPS dosáhneme při kontinuálním běhu
- kdy nás GC pressure z per-cell `Uint32Array` alokací zničí
- kolik let WASM/Rust by reálně přineslo

Prototyp staví na kódu prototypu 5 (3D varianta s 12 opcody, slot model, sub-tick reflow). Bez dominance/intrusion z prototypu 6 - cíl je baseline perf.

## Spuštění

Otevři `index.html` v prohlížeči. Bez build kroku.

## Měření

Panel "Performance" ukazuje:

- **Počet cel**: N³, např. 32³ = 32 768
- **Slotů celkem**: součet `cell.memory.length` přes všechny cely. Mění se s difuzí.
- **RAM odhad**: hrubý odhad paměti. Reálné využití je vyšší kvůli JS object overhead.
- **ms / tik**: rolling průměr posledních 30 tikni
- **FPS**: rolling průměr framerate při kontinuálním běhu

Tlačítko **"Benchmark 100 tiků"** spustí 100 step() bez render-u, naměří wall-clock čas, a vypočítá:
- celkový čas
- ms na tik
- cel/s (kolik cel-tikněti za sekundu engine zvládá)

## Cíle pro porovnání s reálnou implementací

| N | Cel | Slotů (avg) | Předpokládaná baseline (JS) | Předpokládaná Rust+WASM |
|---|-----|-------------|------------------------------|--------------------------|
| 16 | 4096 | 8K | < 5 ms | < 1 ms |
| 32 | 32K | 64K | 20-50 ms | 2-5 ms |
| 64 | 262K | 500K | 200-500 ms | 20-50 ms |
| 100 | 1M | 2M | 1-3 sec | 100-300 ms |

Pokud tabulka roughly odpovídá reálu, **Rust+WASM dá 5-10× zrychlení** - dost na to, abychom mohli running 100³ realtime. JS samo bude limit u ~32-48 N.

## Co (záměrně) chybí

- dominance/intrusion (jen append-to-end Phase 2)
- lineage tracker, war paint, sid/paint opcodes
- asembler/disasembler
- senzory mimo `senergy`

Pokud chceš experimentovat s těmito features, použij prototyp 6 (2D verze). Tady je cíl výhradně: kolik výkonu vytáhneme z JS.

## Známé problémy

- per-cell `Uint32Array` alokace v každém Phase 2 znamená velký GC tlak. Při N > 64 je to už citelné.
- vizualizace 2D řezu zachycuje jen jednu vrstvu - pro 3D simulaci bys ideálně chtěl volumetrické vykreslení (Three.js, WebGL). Pro performance test stačí řez.
- canvas s velkým N má jemné pixely - může být užitečné resize canvas větší.
