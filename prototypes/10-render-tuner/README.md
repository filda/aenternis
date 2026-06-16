# Prototyp 10 - Render tuner

Tournament-style chooser pro parametry render bloku v `/web/`. Statický
svět se vygeneruje jednou (WASM, fixed seed), pak 5x2 mřížka náhledů,
každý s jinou hodnotou laděného parametru. Klik na variantu - hodnota se
zafixuje, přechod do dalšího kola.

## Pořadí kol

| # | Parametr | Rozsah | Default |
|---|----------|--------|---------|
| 1 | `exposure`       | 0.3 - 2.5 | 1.00 |
| 2 | `emissive`       | 0 - 2     | 0.50 |
| 3 | `roughness`      | 0 - 1     | 0.60 |
| 4 | `bloomStrength`  | 0 - 2     | 0.80 |
| 5 | `bloomThreshold` | 0 - 1     | 0.70 |
| 6 | `bloomRadius`    | 0 - 1     | 0.40 |
| 7 | `fogDensity`     | 0 - 0.03  | 0.005 |
| 8 | `ssaoRadius`     | 1 - 20    | 8.0  |
| 9 | `voxelSize`      | 0.2 - 4.5 | 4.50 |
| 10 | `minLuma`       | 0 - 1.00  | 0.42 |
| 11 | `dofMaxblur`    | 0 - 0.05  | 0.000 |
| 12 | `dofFocus`      | 0 - 400   | 50 |
| 13 | `dofAperture`   | 0 - 0.1   | 0.025 |

DoF je defaultně vypnutý (`dofMaxblur = 0` → BokehPass přeskočen), takže kola
1–10 jsou ostrá. `dofMaxblur` jde jako první z DoF kol — zapne pass, pak se
`dofFocus`/`dofAperture` ladí s viditelným rozmazáním. Pozn.: `dofFocus` (max
400) nedosáhne na mračno při fit-zoomu vzdálené ~1900 jednotek — DoF tuning má
u velkých světů omezenou užitečnost (platí i pro viewer).

Po posledním kole se zobrazí JSON s finalní kombinací - lze nakopírovat
do slider defaultů v root `index.html`.

## Statický svět

Snapshot se generuje **deterministicky** podle hardcoded receptu
(`CAPTURE_SEED=1234`, `CAPTURE_ENERGY=1_000_000`, `CAPTURE_TICKS=250`).
Při prvním otevření stránky se počítá za běhu (~1-3s). Tlačítko
**Save snapshot** v topbaru stáhne `snapshot.bin` + `snapshot.meta.json`;
když oba soubory zaarchivuješ vedle `main.ts`, prototyp je při dalším
otevření použije přímo a kapture přeskočí.

### Headless přegenerování (bez prohlížeče)

Snapshot odráží chování core v době captuře, takže časem zastarává
(`./build` staví jen web target). Přegeneruj ho z aktuálního core dvěma
příkazy z rootu repa — `scripts/gen-tuner-snapshot.mjs` zrcadlí stejný
`CAPTURE_*` recept jako browser:

```
wasm-pack build crates/aenternis-wasm --target nodejs --release --out-dir pkg-node
node scripts/gen-tuner-snapshot.mjs
```

První příkaz postaví nodejs-target wasm (`pkg-node/` — jinak `./build`
emituje jen web target a node build zastará), druhý přepíše `snapshot.bin`
+ `snapshot.meta.json` a vypíše počet buněk / bbox nového světa.

## Architektura

- Jeden `THREE.WebGLRenderer` + jedna `PerspectiveCamera` s
  `OrbitControls` (sdílený pohled napříč všemi tiles).
- Každý tile má vlastní `Scene` + `EffectComposer` se svojí instancí
  `SSAOPass` + `BokehPass` (DoF) + `UnrealBloomPass` + `OutputPass`. Voxel
  `InstancedMesh` per tile (sdílená flat-shaded icosahedron geometrie =
  krystal, jako produkční viewer). BokehPass je vždy aktivní (turnaj ho ladí);
  při defaultním `maxblur` je rozmazání jemné, takže ostatní kola zůstávají
  čitelná.
- Skybox env (PMREMGenerator) se peče jen jednou a sdílí napříč tiles.
- Tiles se renderují přes `renderer.setViewport` + `setScissor`
  na jeden velký canvas.

## Spuštění

```
npm run dev
```

Pak `/prototypes/10-render-tuner/` v prohlížeči.
