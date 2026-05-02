# Prototyp 2: proto-entita jako energeticka pamet

Druhy laboratorni prototyp overuje vazbu mezi energii, velikosti pameti, starnutim pametovych bunek a mutaci.

## Spusteni

Otevri `index.html` v prohlizeci. Neni potreba build, npm ani lokalni server.

## Co prototyp dela

- energie entity urcuje pocet dostupnych pametovych bunek
- pri ztrate energie se pamet zkracuje od konce
- program i data jsou zde reprezentovane stejnym polem bajtu
- kazda pametova bunka ma vek
- zapis do bunky resetuje vek
- starsi bunky maji vyssi pravdepodobnost mutace
- mutace preklopi jeden nahodny bit a resetuje vek bunky

## Obnova pameti

Volba `Periodicky obnovovat jadro` simuluje princip `$a = $a`.

Entity nic "noveho" nepise, jen znovu potvrzuje obsah prvnich N bunek. Vek techto bunek se resetuje, takze jadro programu zustava stabilnejsi nez neudrzovany konec pameti.

Tato volba je pouze laboratorni pomucka. Ve finalnim modelu by nemela existovat automaticka ochrana jadra, pokud pro ni nenajdeme fyzikalni pravidlo sveta. Obnova pameti ma byt dusledek programu entity, ne vestavena vyjimka.

## Co sledovat

- pri nenulove ztrate energie se pamet zkracuje od konce
- vypnute obnovovani jadra zvedne pocet rizikovych bunek a mutaci
- zapnute obnovovani chrani nizke adresy
- tlacitko `Ztratit` ukaze okamzite useknuti konce pameti
- tlacitko `Pridat` zvetsi dostupnou pamet, nove bunky se objevi na konci

## Zjednoduseni

Tento prototyp jeste nema virtualni stroj ani Program Counter. Pamet je zatim pouze energeticko-informacni hmota, na ktere se testuje starnuti, obnova a mutace.

## Pozorovani

- Pri vypnute ochrane jadra se model stale chova podle ocekavani.
- Polocas starnuti kolem `1000` taktu vedl v rucnim testu k preziti zhruba `5000` taktu.
- Konkretni cisla nejsou zatim designove zavazna; bude je potreba kalibrovat pozdeji spolu s VM, energetikou a pohybem.
