# Prototyp 8: 3D Viewer

Osmý laboratorní prototyp **se neuvažuje fyzika**. Jeho úloha je výhradně vizualizační: zjistit, **jestli má 3D pohled na Aenternis budoucnost**, nebo zda jsme odsouzeni k 2D řezům jako prototypový debug nástroj.

Klíčové otázky:

- jak vypadá 100³ pole jako voxelová scéna v Three.js
- kolik FPS dosáhneme s 1M voxely (instanced rendering)
- jaké visualization mode dávají hráčsky smysl
- jak se chová OrbitControls kamera v takto hustém prostoru
- co potřebujeme nad rámec stávajícího pro hratelnost

## Spuštění

**Preferovaně přes dev server** (kvůli Web Workeru):

```
npm install      # poprvé
npm run dev:p8   # otevře přímo prototyp 8 na http://localhost:5173/prototypes/08-viewer-3d/
```

Případně otevři `index.html` přímo přes `file://`. V tom případě má prototyp **fallback** pro Web Worker (XHR + Blob URL), ale doporučujeme dev server.

Three.js se natáhne z CDN (unpkg.com).

## Co vidíš

3D scéna s wireframe boundary (drátěný obrys světa) a voxely (kostky) reprezentujícími aktivní cely:

- **Energie (heat)**: cely zbarvené podle energie (inferno paleta: tmavá → červená → oranžová → bílá)
- **Identity (origin tag)**: cely zbarvené podle origin tagu (HSV mapping, hue z tagu)

Kamera je kombinace **OrbitControls** + **WSAD** (FPS-style):
- **drag** = orbit (rotace kolem středu)
- **scroll** = zoom
- **pravé tlačítko + drag** = pan
- **W/S** = dopředu / dozadu po směru pohledu
- **A/D** = doleva / doprava (perpendikulárně k pohledu)
- **Q/E** = dolů / nahoru (world Y)
- **Shift** = sprint (3× rychleji)

WSAD a OrbitControls fungují společně - WSAD posouvá target i kameru o stejný delta, takže orbit zůstane konzistentní.

## Tracker (highlight + trail)

V scéně je sledována **cella s nejvyšší energií**. Po každém ticku:

- okolo aktivní cely se vykreslí žluto-bílý wireframe rámeček (lehce pulsuje, aby šel vidět)
- v posledních N pozicích se vykreslí **trail** - liniová stopa, kde starší body fade-ují do tmavé. Trail se přidá jen když se aktivní cela skutečně posunula (jinak by se hromadily duplicity ve stejném bodě).
- v HUDu se zobrazuje aktuální `(x, y, z)` pozice a energie

Trail délka je nastavitelná sliderem (0 = vypnutý trail, 200 = dlouhá historie).

## Web Worker mód

Checkbox **Web Worker** přepne simulaci do background threadu (`worker.js`). Tím se uvolní render thread - i při běhu simulace 100³ by render mohl běžet na 60 FPS, zatímco worker pumpuje state asynchronně.

- Reset / Krok / Spustit / Coeff slider posílají zprávy workerovi přes `postMessage`
- Worker po každém ticku posílá zpět `Uint32Array` energií jako Transferable - žádné kopírování, jen předání ownership
- Origin tagy se posílají jednorazově po resetu (statické po dobu scénáře)

**Pozor**: Chrome blokuje `new Worker("worker.js")` z `file://` (null origin). Skript proto má **fallback** - pokud konstruktor selže, načte worker.js přes XHR a obalí do Blob URL. To obvykle obejde restrikci. Pokud i to selže, prototyp se gracefully přepne na main-thread mód a v HUDu se zobrazí "Web Worker: nedostupný".

## Výkonostní triky

- **InstancedMesh** - jeden SphereGeometry (8×6 segmentů), N³ instancí. Klíčové pro 1M voxelů.
- **Per-instance color** přes `instanceColor` attribute
- **Hide empty cells** přes scale=0 v matrix - "neviditelné" cely netáhnou pixel work
- **Min energy slider** - ručně schovat slabé cely (vidět jen "vysoce aktivní")
- **HUD update sampling** - metriky se updatují jen každých 100ms
- **Decoupled render/sim loops** - render běží v `requestAnimationFrame`, sim ve vlastní async smyčce (nebo Web Workeru). State.dirty flag synchronizuje předávání stavu.
- **Async chunked step** (main-thread mód) - simulace yielduje na browser každých 4000 cel, takže render má šanci proběhnout

## Co (záměrně) chybí

- žádný inspector cely (klik na voxel)
- žádné CPU vykonávání instrukcí (jen difuze)
- žádná dominance/intrusion (jednoduchá Phase 2 jako prototyp 5)
- žádný asembler / inject programů
- tracker sleduje jen "max energy cell", ne uživatelem zvolenou entitu

Cíl je ČISTĚ vizualizace - posoudit, jestli stojí za to pokračovat tímto směrem.

## Otázky k zodpovědění po experimentu

1. **Performance**: kolik N je realisticky plynule renderovatelné? (Hledáme threshold kde FPS spadne pod 30.)
2. **Čitelnost**: dá se v 3D voxelovém poli orientovat? Nebo je to jen "krásná blob"?
3. **Hratelnost**: viděl by hráč svou entitu, nebo by se ztratila v davu?
4. **Camera UX**: stačí OrbitControls, nebo by chtěl FPS-style kamera (WASD pohyb)?
5. **Min energy filter**: pomáhá zvýraznit aktivní oblasti, nebo skrývá důležitý kontext?
