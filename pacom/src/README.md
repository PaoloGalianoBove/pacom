# PACOM: Platform Agnostic COmmunication Middleware

Benvenuto nel codice sorgente di **PACOM**. Questo file e i vari `README.md` presenti nelle sottocartelle hanno l'obiettivo di abbassare drasticamente la barriera d'ingresso per i contributor open-source. Leggendo queste guide, capirai l'architettura interna, le scelte di design e potrai iniziare a contribuire immediatamente.

## Architettura del Progetto

PACOM è un middleware progettato per semplificare l'utilizzo di **uProtocol** (uno standard automotive per la comunicazione agnostica). uProtocol di per sé è molto verboso e complesso; PACOM agisce come un wrapper "opinionated" che fornisce un'interfaccia idiomatica in Rust.

Il progetto è diviso rigidamente in tre layer:

1. **`public_api/` (Interfaccia Utente)**: 
   È la facciata pubblica. Le applicazioni importano i metodi definiti qui per comunicare (`invoke_method`, `publish_event`). Questo livello lavora solo con *stringhe logiche* (es. `"/rpc/echo"`) e payload in bytes. Non conosce l'esistenza degli URI formali di uProtocol o dei trasporti fisici.

2. **`runtime/` (Il Cervello del Middleware)**:
   È il motore (engine) che orchestra tutto. Quando l'utente chiama `publish("/topic")`, il `runtime` converte questa stringa in un URI numerico (attraverso il mapping definito in `logical_registry.rs`), costruisce un messaggio formale uProtocol (`UMessage`), e lo passa al layer sottostante per l'instradamento. Qui vivono anche i client/server RPC in-memory e il meccanismo di Discovery.

3. **`transport/` (I Bridge Fisici / uTransport)**:
   Questo layer si occupa di inviare fisicamente i byte sulla rete. Contiene implementazioni del trait `UTransport` di uProtocol.
   - **`vsomeip.rs`**: Per la comunicazione ultra-veloce intra-veicolo (SOME/IP).
   - **`mqtt.rs`**: Per la comunicazione cloud/off-board (MQTT 5).
   - **`router.rs`**: Implementa il concetto di *uStreamer*. Prende un messaggio uProtocol e decide se inviarlo via vSomeIP o via MQTT analizzando l'autorità di destinazione.

## Flusso di vita di un Messaggio (Esempio RPC)

Per capire come contribuire, ecco cosa succede quando un'applicazione chiama `runtime.invoke_method("/rpc/echo", payload)`:

1. La chiamata parte dal **`public_api`**.
2. Arriva nel **`runtime::RuntimeEngine`**.
3. L'engine usa il **`logical_registry`** per cercare se `"/rpc/echo"` è nel `manifest.json`. Se c'è, genera l'`UUri` (URI di uProtocol) numerico univoco (es. `0x1234.8001`).
4. L'engine consulta la sua cache di **Discovery** per trovare quale container/ECU (identificata dall'`UE_ID` provider) offre questo servizio. Se non c'è, fa polling attendendo che appaia sulla rete.
5. Una volta noto l'indirizzo del provider, l'engine usa `InMemoryRpcClient` per serializzare il messaggio in un pacchetto Request uProtocol.
6. Il messaggio viene inviato al **`transport::router`** (UStreamerRouter).
7. Il router legge l'authority di destinazione. Se è locale/veicolare, passa il messaggio al trasporto **`vsomeip`**.
8. Il trasporto `vsomeip` fa il bind con la libreria C++ sottostante e invia i byte sul socket IPC o sulla rete Ethernet (multicast/unicast).

## Regole d'oro per contribuire

- **Non esporre uProtocol all'esterno**: Qualsiasi modifica fai in `runtime` o `transport`, assicurati che le strutture di uProtocol (come `UMessage`, `UUri`, `UStatus`) non "filtrino" mai nell'API pubblica (tranne all'interno di `PacomError`).
- **Error Handling**: Usa sempre `PacomError` definito in `src/error.rs`. Non usare `panic!` o `unwrap()` nel codice del middleware, poiché un middleware non deve mai far crashare l'applicazione utente.
- **Isolamento dei Thread**: Quando apri thread in background (es. per il discovery), assicurati che non mantengano lock pesanti o blocchino il ciclo di vita dell'applicazione.

Vai ora nelle sottocartelle `public_api/`, `runtime/` e `transport/` e leggi i relativi `README.md` per scendere nel dettaglio del codice.
