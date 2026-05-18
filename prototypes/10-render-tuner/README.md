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
| 9 | `voxelSize`      | 0.2 - 1.5 | 1.00 |

Po posledním kole se zobrazí JSON s finalní kombinací - lze nakopírovat
do slider defaultů v `web/index.html`.

## Statický svět

Snapshot se generuje **deterministicky** podle hardcoded receptu
(`CAPTURE_SEED=1234`, `CAPTURE_ENERGY=1_000_000`, `CAPTURE_TICKS=250`).
Při prvním otevření stránky se počítá za běhu (~1-3s). Tlačítko
**Save snapshot** v topbaru stáhne `snapshot.bin` + `snapshot.meta.json`;
když oba soubory zaarchivuješ vedle `main.ts`, prototyp je při dalším
otevření použije přímo a kapture přeskočí.

## Architektura

- Jeden `THREE.WebGLRenderer` + jedna `PerspectiveCamera` s
  `OrbitControls` (sdílený pohled napříč všemi tiles).
- Každý tile má vlastní `Scene` + `EffectComposer` se svojí instancí
  `SSAOPass` + `UnrealBloomPass` + `OutputPass`. Voxel `InstancedMesh`
  per tile (sdílená sphere geometrie).
- Skybox env (PMREMGenerator) se peče jen jednou a sdílí napříč tiles.
- Tiles se renderují přes `renderer.setViewport` + `setScissor`
  na jeden velký canvas.

## Spuštění

```
npm run dev
```

Pak `/prototypes/10-render-tuner/` v prohlížeči.
