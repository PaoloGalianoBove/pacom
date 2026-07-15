# Modulo `runtime`

Il modulo `runtime` è la sala macchine di PACOM. È la colla che unisce le stringhe dell'API pubblica con le strutture dati rigide di uProtocol. Chiunque voglia fare debug su "perché un messaggio non parte" o "perché la discovery non va" dovrà guardare qui.

## `engine.rs`

Questo file contiene l'`Engine`, ovvero lo stato principale dell'applicazione.
L'engine detiene:
- Il `ManifestConfig` dell'applicazione.
- L'istanza del `router` uStreamer (per smistare verso vsomeip o mqtt).
- Le istanze di `InMemoryRpcClient` e `InMemoryRpcServer` (forniti dalla libreria ufficiale `up-rust`), necessari per gestire l'asincronia delle chiamate RPC.
- Una tabella di cache concorrente (es. `DashMap` o un `RwLock<HashMap>`) per il **Service Discovery**.

### Il ciclo di Discovery (Come le app si trovano tra loro)

Il discovery di PACOM è proprietario e basato su UDP/vSomeIP PubSub (non usa lo standard `uDiscovery` per semplicità). Funziona tramite due task in background (tokio spawned tasks) inizializzati dentro `PacomRuntime::new`:

1. **Il Broadcaster (Annuncio)**:
   In un ciclo infinito (ogni `PACOM_DISCOVERY_REANNOUNCE_SECS` secondi), l'engine prende tutti gli RPC `provide` e i topic `publish` dal `ManifestConfig` e costruisce dei messaggi JSON (es. `{ "kind": "rpc_provide", "name": "/rpc/echo", "provider_ue_id": 4660 }`). Invia questi JSON su un topic hardcoded noto a tutti (URI `0x0F00.8F01`).

2. **L'Ascoltatore (Cache)**:
   Al boot, l'engine si iscrive al topic `0x0F00.8F01`. Ogni volta che riceve un JSON da un'altra applicazione sulla rete, fa il parsing e aggiorna la sua cache interna: *"Ah, il metodo `/rpc/echo` si trova sull'app con UE_ID `0x1234`!"*.

Quando chiami `invoke_method("/rpc/echo")`, la funzione va a leggere in questa cache. Se il provider per `/rpc/echo` non è ancora arrivato, la funzione fa un loop di `sleep` (polling) fino a quando non appare o scade il timeout (180 secondi di default).

## `logical_registry.rs`

L'obiettivo di uProtocol è indirizzare i messaggi usando interi (URI numerici) per risparmiare byte sul bus. L'utente PACOM usa invece nomi stringa. Come li mappiamo?

1. **Il `ManifestConfig`**:
   Legge il file `manifest.json`. È lo schema che certifica cosa l'applicazione ha il permesso di fare. Le liste `rpc.provide`, `rpc.consume`, `topics.publish`, `topics.subscribe` vengono popolate qui.

2. **L'Algoritmo di Hashing FNV-1a**:
   Invece di avere un registro centrale che assegna ID arbitrari (come `echo = 1`, `telemetry = 2`), PACOM calcola dinamicamente l'ID usando l'algoritmo di hash FNV-1a sulla stringa logica.
   - Per un topic (es. `/sensors/speed`), fa l'hash e forza l'ID nel range `0x8000 - 0xFFFF` (range previsto per le risorse/eventi).
   - Per un metodo RPC (es. `/rpc/echo`), fa l'hash e forza l'ID nel range `0x0001 - 0x7FFF` (range previsto per i metodi).

3. **Collision Detection (Sicurezza)**:
   L'hash FNV-1a a 16 bit può avere collisioni (due stringhe diverse che generano lo stesso numero).
   All'avvio, `logical_registry` scansiona tutte le stringhe nel manifest, ne calcola gli ID, e verifica che non ci siano due stringhe che producano lo stesso ID. Se succede, restituisce un `PacomError::IdCollision` impedendo il boot dell'applicazione ed evitando bug di indirizzamento critici a runtime.

## Come contribuire a `runtime`

- **Performance**: Il discovery polling loop in `invoke_method` potrebbe essere ottimizzato passando da uno `sleep` in polling a un meccanismo di `Notify` o `watch` asincrono fornito da tokio.
- **Conformità uProtocol**: Se vuoi implementare la specifica ufficiale `uDiscovery` protobuf-based, questo è il posto giusto. Dovrai rimpiazzare il builder JSON con messaggi protobuf e cambiare i ruoli di `register`/`resolve`.
