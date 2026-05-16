# Native dev backend

Aenternis viewer má dva backendy, které sdílejí jeden protokol:

- **WASM Web Worker** (default) — `wasm-pack` kompiluje
  `aenternis-wasm` do prohlížeče, simulace běží uvnitř Web Workeru
  v izolovaném tabu. Bez serveru, bez sítě, deployable na GitHub
  Pages. Typicky 1–2 ms/tick na malých světech, 1 vlákno.
- **Native dev backend** — Rust binárka `aenternis-server`, která
  drží **sdílený svět** a vystavuje ho přes WebSocket. Viewer
  (vite-dev nebo deployed) se k ní připojí a chová se jinak ne
  než ve WASM cestě. Cílem je rychlejší tick (rayon přes všechna
  jádra, nativní LLVM), kratší rebuild loop (`cargo run` místo
  `wasm-pack`) a možnost si postavit u stolu lab kde víc tabů
  vidí stejnou simulaci.

Tenhle dokument je dev runbook: jak to spustit, jak to přepnout,
co očekávat.

## Spuštění serveru

Ze **sandbox clone** (ne live mount — `target/` nesmí mixovat
linux a windows artefakty, viz `AGENTS.local.md`):

```sh
bash scripts/server.sh
```

Wrapper je analog `./check` — na Windows (Git Bash /
MSYS2) přidá `+stable-x86_64-pc-windows-msvc`, jinde použije
default cargo. Bez něj plain `cargo run -p aenternis-server`
táhne `rust-toolchain.toml` na host triple gnu, který v
`windows-sys` linkování chce `dlltool.exe`.

Alternativně jednou ranou s Vite dev serverem:

```sh
./run --server
```

Spustí oba procesy paralelně ve stejné procesní skupině; Ctrl+C
zastaví oba najednou.

Argumenty se pass-through na binárku:

```sh
bash scripts/server.sh --host 0.0.0.0 --port 9000
```

Default bind `127.0.0.1:8765`. Výstup:

```
INFO aenternis-server listening on http://127.0.0.1:8765 (ws at /sim)
```

Server drží jednu globální `SparseWorld` instanci. Klienti, kteří
se připojí na `ws://127.0.0.1:8765/sim`, sdílejí stejný stav —
init/config/running/step od kteréhokoliv klienta vidí všichni.

### Konfigurace

| Flag           | Env var          | Default     | Význam                                  |
|----------------|------------------|-------------|------------------------------------------|
| `--host HOST`  | `AENTERNIS_HOST` | `127.0.0.1` | Bind adresa. `0.0.0.0` pro LAN exposure. |
| `--port PORT`  | `AENTERNIS_PORT` | `8765`      | TCP port.                                |
| `-h`/`--help`  |                  |             | Vypíše usage a exit 0.                   |

Příklad pro test z jiného stroje na LAN:

```sh
bash scripts/server.sh --host 0.0.0.0 --port 8765
```

Server v tomto případě vypíše varování:

```
WARN bound to non-loopback address — no auth enforced. LAN-only, dev use.
```

Bez autentizace. Použít jen na důvěryhodné síti.

### Endpointy

- `GET /health` — plain text `ok`. Sanity check že server běží
  (`curl http://localhost:8765/health`).
- `GET /sim` — WebSocket upgrade. Po handshake server pošle
  `{"type":"ready"}` JSON, hned potom `{"type":"welcome","running":<bool>}`,
  a pak začne broadcastovat snapshoty (binární frames, tag 1).
  CellDetail (binární frames, tag 2) chodí jen ke klientovi,
  který poslal `inspect`.

### Vypnutí

`Ctrl-C`. Graceful shutdown — axum dokončí in-flight responses
a vrátí kontrolu shellu.

## Přepnutí backendu ve viewer

Default je WASM. Přepnutí na native:

**URL flag** (jednorázové, ale persistentní — uloží se do
`localStorage`):

```
http://localhost:5173/web/?backend=native
```

URL flag taky umí override server URL:

```
http://localhost:5173/web/?backend=native&server=ws://192.168.1.42:8765/sim
```

**Checkbox v UI** — v boční hud panelu je sekce "Backend" s
checkboxem "native dev backend". Změna stavu uloží do
`localStorage` a reloadne stránku.

**localStorage klíče** (kdyby ses chtěl podívat):

- `aenternis.backend` — `"wasm"` nebo `"native"`
- `aenternis.server` — WebSocket URL, např. `"ws://localhost:8765/sim"`

Default URL pro native je `ws://<window.location.hostname>:8765/sim`,
takže když Vite spustíš s `--host` a viewer otevřeš z LAN IP,
WS se připojí na ten samý hostname.

## Sdílený svět — co to znamená

Server má jednu `SparseWorld`. **Každý** control message
(init/config/running/step/inspect) z **kteréhokoliv** klienta
se aplikuje na ten samý svět. Důsledky:

- **Reset z tabu A vidí tab B.** Tab B uvidí na příštím
  snapshotu tick=0. Pokud měl rozeditovaný program, server ho
  nenese — program existuje jen v paměti origin cellu, kterou
  reset přepíše. Tj. `programText` v UI tabu B je teď **lež**
  (UI ukazuje text, který byl použit pro init A); ošetři si to
  v hlavě nebo si to znovu vlož a Reset.
- **Pause/Resume z tabu A vidí tab B.** Po pause flag se
  propaguje přes `WelcomeMsg` jen na nově připojené klienty
  (existující klienti to poznají z toho, že přestaly chodit
  snapshoty). Pause button v tabu B nezmění label dokud sám
  neklikne Pause — to je rys, ne bug, do budoucna se dá doplnit
  `RunningChangedMsg` broadcast.
- **Config (coeff/k/moveThreshold) z tabu A vidí tab B v
  chování světa**, ale slidery v B se neaktualizují (stejná
  věc jako u Pause).

Single-tab použití: žádný rozdíl proti WASM. Multi-tab jen pro
když to opravdu chceš sdílet, ne pro paralelní experimenty.

## Vrácení do WASM

URL `?backend=wasm` nebo checkbox v UI. Reload zpracuje. WASM
cesta nepotřebuje server, zůstane funkční pro deployed Pages
build i pro `npm run dev` bez `cargo run`.

## End-to-end repro

Smoke test, že to celé žije:

1. Terminál 1: `bash scripts/server.sh`. Čekej na
   `listening on http://127.0.0.1:8765`.
2. Terminál 2: `npm run dev` (sandbox clone, ne live mount —
   AGENTS.local.md). Vite pojede na `http://localhost:5173`.
3. Prohlížeč: `http://localhost:5173/web/?backend=native`.
4. V hud panelu "Backend" by mělo být checkbox zaškrtnutý a
   URL `ws://localhost:8765/sim`.
5. Resume button → svět se rozjede, ticky letí. V terminálu 1
   nic dramatického (server netracuje per-tick logy v default
   filtru).
6. Otevři druhý tab na stejné URL — měl by ihned vidět běžící
   svět, Pause button v "Pause" stavu (`WelcomeMsg.running:true`).
7. V druhém tabu klikni Pause — oba taby zastaví na stejném ticku.
8. Klikni na buňku v viewer — Inspector se otevře a ukáže její
   stav. (CellDetail je per-client — druhý tab nic neuvidí.)
9. Reset v jednom tabu → druhý tab uvidí tick=0 na příštím
   snapshotu.

Pokud cokoli z toho selže, log serveru (`RUST_LOG=debug bash
scripts/server.sh`) ukáže commandy, které dorazily.
F12 v prohlížeči ukáže WS framy v Network → WS.

## Performance očekávání

Tahle sekce čeká na první konkrétní benchmark. Hrubý odhad:

- **Tick čas** — native release build s rayon na 8 jádrech
  oproti WASM single-thread typicky 2–4× rychlejší na střední
  a velké světy (1k+ buněk). Pro 100 buněk tick je už tak
  ULP-malý, že rozdíl tone v noise.
- **Wire overhead** — loopback WS RTT typicky < 200 µs, snapshot
  ~24 B na buňku. Pro 100k buněk = 2.4 MB/snapshot, při 60
  Hz = 144 MB/s. Loopback to zvládá, LAN gigabit sotva.
- **Iterace** — tohle je největší win. `cargo run` má cca
  sekundový rebuild proti 5–15 s `wasm-pack build`. Pro
  fyzikální experimenty kde měníš `tick.rs` po každém runu
  to dělá rozdíl.

## Implementační poznámky pro budoucího sebe

- Wire format je definovaný na dvou místech: server v
  `crates/aenternis-server/src/protocol.rs`, klient v
  `src/native-client.ts`. Když se mění layout, oba musí jet
  zároveň. JSON část je deklarativní (serde + TS interface),
  binární část ručně přes `to_le_bytes`/`DataView` —
  případnou chybu off-by-one chytí round-trip testy v obou
  modulech.
- Snapshot fan-out přes `tokio::sync::broadcast` s
  `Arc<Vec<u8>>` — encode jednou, broadcast všem klientům
  zero-copy. Cap 64; lagging klienti tichou ztrátu starých
  snapshotů, viewer beztak renderuje jen poslední.
- Inspect přes `oneshot::Sender` v Command — reply chodí jen
  k volajícímu klientovi, ne přes broadcast. Vyhneme se tím
  paintování cizích inspektorů.
- World actor je single tokio task. CPU-bound `tick::step`
  blokuje 1 worker až do dokončení, ale rayon uvnitř step
  přesto rozdistribuuje výpočet přes pool. Pokud by se to
  v praxi ukázalo jako bottleneck, dá se přesunout na
  `tokio::task::spawn_blocking` per-tick.
- `welcome` state je v `tokio::sync::watch` — cheap peek z
  WS handleru bez nutnosti prvního event subscribe loopu.
