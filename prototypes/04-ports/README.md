# Prototyp 4: porty, raketovy pohyb a sani energie

Ctvrty laboratorni prototyp overuje entitu jako raketu se smerovymi porty.

## Spusteni

Otevri `index.html` v prohlizeci. Neni potreba build, npm ani lokalni server.

## Model

- svet je 3D torus s difuzi energie
- entita je samostatny energeticky objekt v jedne bunce
- cteni portu je zatim zobrazeno jako hodnota energie v sousedni bunce
- zapis do portu je `Zazeh`
- zazeh odebere energii entite, vyzari ji do sousedni bunky ve smeru portu a posune entitu opacnym smerem
- presun se povede jen pokud je energie cilove bunky dost nizka v pomeru k energii entity
- po uspesnem presunu entita absorbuje vsechnu energii v cilove bunce
- `Sat` aktivne odcerpa energii ze sousedni bunky do entity
- volitelne muze sani pritahovat entitu smerem ke zdroji
- unik entity za takt pomalu vyzaruje energii do vsech sesti smeru

## Co sledovat

- silny zazeh zkrati pamet entity, protoze pamet je odvozena z energie
- spatny manevr muze spalit vic energie, nez entita ziska absorpci cilove bunky
- nizky `Prah pohybu cil / entita` znamena pohyb skoro jen do prazdna
- vyssi prah dovoli huste entite projit pres slabou "susinku"
- pri pruchodu horkou oblasti se entita muze posilovat tim, ze uspesne absorbuje cilove bunky
- pokud se zasekne na okraji oblasti, znamena to, ze dalsi cilova bunka uz prekrocila pomerovy prah vuci energii entity po zazehu
- sani muze slouzit jako zpusob lovu energie z okoli
- vyzarena energie zustava ve svete a dale difunduje
- pohyb pres okraj prostoru se pretaci na opacnou stranu

## Zjednoduseni

Tento prototyp jeste nema VM instrukce pro porty. Porty se ovladaji rucne tlacitky. Pohyb je diskretni: jeden zazeh znamena jeden posun o bunku opacnym smerem. Pozdeji muze vzniknout plynulejsi model s impulsem nebo rychlosti.
