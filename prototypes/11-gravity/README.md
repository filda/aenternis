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
- **Hranice = void** (E=0). Energie, která vyteče ven, je **ztracená** —
  metrika `Ztraceno (void)` ukazuje napětí "struktury vs. tepelná smrt".
- **Šum** drobně rozkmitá drivy (analog `stochastic_floor` v jádru) a láme
  symetrii, aby z dokonale symetrického počátku mohly vyrůst struktury
  (Jeansova nestabilita).

## Scénáře

- **Velký třesk** — všechna energie v jedné buňce (max hustota). Čeká se:
  tlak ji rozmetá (exploze) → jak hustota klesá, v přehuštěných kapsách
  začne vyhrávat gravitace → shluky.
- **Šum** — náhodné pole; přímo testuje, jestli gravitace zesílí
  fluktuace do shluků.
- **Dvě tělesa** — sledovat, jestli se přes void přitáhnou.

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
