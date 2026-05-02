# Prototyp 1: difuze energie v 3D toru

První laboratorní prototyp ověřuje jen tok energie v diskrétním 3D toru.

## Spuštění

Otevři `index.html` v prohlížeči. Není potřeba build, npm ani lokální server.

## Co prototyp dělá

- vytvoří 3D mřížku s periodickými okraji
- vloží všechnu energii do jedné centrální buňky
- v každém taktu spočítá tok do šesti sousedů podle rozdílu potenciálů
- zobrazuje 2D řez světem jako heatmapu
- měří celkovou energii, drift, maximum, minimum, průměr a rozptyl

Použitý krok:

```text
next = current + rate * (sum(sousedi) - 6 * current)
```

Pro stabilitu by `rate` neměl být větší než přibližně `1/6`.

## Co sledovat

- Celková energie by měla zůstat téměř konstantní.
- Drift by měl být jen numerická chyba.
- Maximum by mělo klesat.
- Rozptyl by měl klesat směrem k rovnoměrnému stavu.
- Nerovnováha je `směrodatná odchylka / průměr`; rovnovážný stav se blíží nule.
- Vychladlo ukazuje první takt, kdy nerovnováha klesla pod nastavený práh.
- Při větších světech by počáteční hustota měla působit jako prudký rozpad do okolí.

## Pozorování

- `32^3`, energie `1 000 000`, tok `0.080`, práh `0.010`: vychladnutí kolem taktu `1788`.
- `100^3`, energie `1 000 000`, tok `0.080`, práh `0.010`: vychladnutí kolem taktu `17422`. Žhavá oblast zůstává dlouho kompaktní, což odpovídá většímu prostoru.

Poměr vychladnutí `100^3 / 32^3` je přibližně `9.7x`, což zhruba odpovídá intuici, že difuzní čas roste s druhou mocninou rozměru prostoru.

Po dosažení téměř rovnoměrného stavu může být vidět drobné numerické chvění nerovnováhy. Nejde o fyzikální kmitání světa, ale o numerickou oblast velmi blízko nule.

## Poznámka k barvám

Výchozí barevné měřítko je `Prumer = stred`. Rovnovážný stav se proto nevykreslí jako plné maximum, ale jako střední hustota.

Volba `Aktualni maximum` škáluje barvy podle nejhustší buňky v aktuálním taktu. Je užitečná pro sledování tvaru rozptylu, ale v rovnováze je matoucí: všechny buňky mají stejnou hodnotu, takže jsou všechny zároveň aktuálním maximem.
