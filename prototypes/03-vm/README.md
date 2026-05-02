# Prototyp 3: minimalni VM proto-entity

Treti laboratorni prototyp pridava Program Counter a jednoduchou instrukcni sadu nad spolecnou pameti programu a dat.

## Spusteni

Otevri `index.html` v prohlizeci. Neni potreba build, npm ani lokalni server.

## Model

- pamet ma zatim pevny adresni prostor `0x00-0xff`
- energie urcuje, kolik adres je zive dostupnych
- pri ztrate energie se pamet zkracuje od konce
- cteni, zapis i PC se adresuji modulo aktualni velikosti zive pameti
- nulova pamet nezpusobi pad; VM tikuje naprazdno a muze se znovu rozbehnout, pokud se energie vrati
- Program Counter muze najet do dat
- zapis do pameti resetuje vek bunky
- starnuti a mutace mohou menit program za behu
- `0x00` neni `halt`, ale necinny bajt/data s delkou 1
- nezname opkody se chovaji jako data/instrukce s delkou 1, aby bylo videt, jak PC prochazi sumem

## Instrukce

```text
00       nop/data
01       nop
10 a v   set [a], v
11 a b   copy [a], [b]
20 a b   add [a], [b]
21 a b   sub [a], [b]
30 a     inc [a]
31 a     dec [a]
40 a     jmp a
41 a b   jz [a], b
```

## Ukazky

- `Nekonecna smycka`: stale inkrementuje jednu datovou adresu a kopirovanim `copy [0], [0]` obnovuje zacatek programu.
- `Self-modifying code`: meni operand vlastni instrukce.
- `Krehky program`: saha na vysoke adresy, takze ztrata energie ho rychle poskodi.
- `PC najede do dat`: pripravi instrukce jako data a skoci do nich.
- `Nahodna pamet`: necha PC prochazet nahodnym obsahem.

## Co sledovat

- mutace opkodu mohou zmenit tok programu
- ztrata energie muze odriznout adresy, na ktere program saha
- vysoke adresy se po zmenseni pameti zacnou skladat modulo do nizsich adres
- zapis puvodne mireny na konec pameti muze po zmenseni prepisovat jadro
- self-modifying programy resetuji stari menenych bunek
- PC v datech nemusi hned znamenat smrt; muze vzniknout nahodne platne chovani

## Zjednoduseni

Zatim chybi porty, pohyb, stackove instrukce a realne oddeleni taktu sveta od instrukci VM. Cilem je jen overit, ze spolecna pamet kodu a dat zacina produkovat zajimave poruchy.

`HLT` zatim neni soucasti instrukcni sady. Bez preruseni by zastavena entita byla jen cihla, takze zastaveni VM je prozatim jen laboratorni stav pri chybe.

Adresovani mimo aktualne dostupnou pamet uz neni chyba. VM pouzije modulo velikosti zive pameti. Pokud je napriklad dostupnych jen `0x80` bunek, adresa `0xf8` se chova jako `0x78`. V trace se to zobrazi jako `0xf8->0x78`.

Pokud dostupna pamet klesne na nulu, VM nespadne. Takt dal probiha jako `idle bez pameti`. Jakmile entita znovu ziska energii a tim i pamet, PC se opet prelozi modulo novou velikosti pameti a program muze pokracovat z toho, co se v pameti objevi.
