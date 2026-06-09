# Prototyp 11 - Gravitace a tlak

Samostatná JS reimplementace jádra (jako prototypy 01-09), která k difuzi
přidává dvě soupeřící síly. Cíl: zjistit, jestli zavedení gravitace/hustoty
vede ke **vzniku struktur** (shluků), nebo se všechno jen rozředí do voidu.

## Model

Hustá 3D mřížka, energie jako `Float64`. Vše je **tok přes stěny** mezi
šesti sousedy. Pro směr `A -> B`:

```
drive(A->B) =  coeff * (E_A - E_B)        // radiace: z kopce dolů (anizotropní)
            +  ( Π(E_A) - Π(E_B) )        // tlak: dolů po tlaku (izotropní)
            +  grav  * (M_B - M_A)        // gravitace: k hmotě (izotropní)
out(A->B)   =  max(0, drive) * dt
```

- **Hmota** `m = α·E` (α složeno do `grav`), takže hustota ∝ energie.
- **Gravitační potenciál** je dlouhodosahový: `M[x] = Σ_{0<|d|≤R} E[x+d] / |d|`
  (jádro `1/r` ⇒ síla `~1/r²`). Počítá se s **cutoffem `R`** kvůli ceně
  (naivně O(N·R³) na tick).
- **Tlakový potenciál** `Π(E) = pressure · eref · (E/eref)^γ` je strmý
  v hustotě (γ>1): při extrémní hustotě přetlačí gravitaci a zastaví kolaps,
  při průměrné hustotě je to jen mírná korekce. `eref` = počáteční průměr.
- Odtoky každé buňky se **proporcionálně ořežou** na její energii ⇒ nikdy
  nejde do záporu a tok je konzervativní.
- **Hranice** je přepínatelná:
  - **Torus** (default) — periodická, nic se neztrácí, přísná konzervace.
    Standardní volba pro studium vzniku struktur (okraj nekazí výsledek).
  - **Void** (E=0) — otevřený vesmír; energie, která vyteče ven, je
    **ztracená**. Metrika `Ztraceno (void)` ukazuje napětí
    "struktury vs. tepelná smrt".
- **Šum** drobně rozkmitá drivy (analog `stochastic_floor` v jádru) a láme
  symetrii, aby z dokonale symetrického počátku mohly vyrůst struktury
  (Jeansova nestabilita).

## Scénáře

- **Velký třesk** — všechna energie v jedné buňce (max hustota). Čeká se:
  tlak ji rozmetá (exploze) → jak hustota klesá, v přehuštěných kapsách
  začne vyhrávat gravitace → shluky.
- **Velký třesk (zrnitý)** — hustý centrální shluk s náhodnými fluktuacemi.
  Mírnější exploze než bodový zdroj + rovnou nasazené semínko fragmentace.
- **Šum** — náhodné pole; přímo testuje, jestli gravitace zesílí
  fluktuace do shluků.
- **Dvě tělesa** — sledovat, jestli se přitáhnou.

## Co rozhoduje (ověřeno headless, 200 taktů, 32³)

- **Void hranice je hlavní úskalí.** S voidem energie radiuje k okraji a
  utíká → ze šumu mizí energie a třesk zůstane jako jedna chladnoucí koule
  (obal uteče, gravitace drží jádro jako jediný důl). **Na toru se třesk
  sám roztříští** (~3400 shluků i s výchozími silami, 0 % ztrát).
- **Škála shluků se ladí silou gravitace** vůči radiaci:
  - slabá gravitace / silná radiace → mnoho mírných shluků (~3500, max ~500),
  - silná gravitace / slabá radiace → méně, ale hustších a hmotnějších
    (~500-800, max ~3000-4000).
- **Šum vs. třesk:** uniformní šum potřebuje, aby gravitace zvítězila od
  nuly (default síly → vyhladí se, 0 shluků; nutná silná gravitace + nízký
  coeff). Třesk/zrnitý třesk mají náskok — koncentrace už tvoří hluboký
  důl, takže fragmentují i s výchozími silami.

## Co ladit

Klíčová je **stavová rovnice**: kde se protnou křivky `grav(ρ)` a tlak `Π(ρ)`.
- Gravitace > tlak všude ⇒ runaway kolaps.
- Tlak > gravitace všude ⇒ jen rozpínání/ředění (tepelná smrt).
- Kříží se ⇒ exploze nahoře + shluky dole (žádaný režim). Hustota křížení
  ≈ charakteristická velikost vzniklých struktur.

Pomalejší pokles (větší `R`) = delší dosah = silnější sklon ke kolapsu, ale
dražší výpočet.

## Spuštění

```
npm run dev
```

Pak `/prototypes/11-gravity/` v prohlížeči.
