# Modulo Public API (`public_api/`)

Questo modulo costituisce il punto d'ingresso principale (entrypoint) per gli sviluppatori di applicazioni. Nasconde la complessità della logica asincrona di uProtocol dietro interfacce user-friendly e tipizzate.

## Componenti Principali

### `PacomRuntime`
L'oggetto centrale del sistema (spesso racchiuso in un `Arc<PacomRuntime>` nei thread applicativi). Inizializza l'orchestrazione locale e funge da proxy per tutti i trasporti.

#### Funzioni Esposte:

1. **`new(config: RuntimeConfig) -> Result<Self, PacomError>`**
   - Inizializza il runtime. Esegue il parsing di `manifest.json`, imposta la `DiscoveryCache`, istanzia il `PacomRouter` e avvia il demone `vSomeIP` e il bridge `MQTT` (se configurati).
   - In caso di collisione di nomi logici nel manifest, ritorna un `PacomError::IdCollision`.

2. **`subscribe_event(&self, topic: &str, callback: impl Fn(Vec<u8>))`**
   - Registra una closure da eseguire in modo asincrono (fire-and-forget) ogni volta che un messaggio `Publish` viene ricevuto sul topic logico specificato.
   - Nasconde l'uso del trait `UListener` instanziando internamente un `ClosureListener`.

3. **`publish_event(&self, topic: &str, payload: Vec<u8>)`**
   - Invia un payload binario (event) al topic logico specificato in modalità broadcast. Il `PacomRouter` deciderà se instradarlo su vSomeIP o MQTT (o entrambi).

4. **`register_rpc_method(&self, method: &str, handler: impl Fn(Vec<u8>) -> Future)`**
   - Registra un servizio di Request/Response. La closure riceve il payload della richiesta e deve restituire una `Future` che risolve nel payload di risposta.
   - Si aggancia al `RequestHandler` standard di `up-rust`.

5. **`invoke_method(&self, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, PacomError>`**
   - Chiama un servizio RPC. Questa funzione attende prima che il servizio sia stato "scoperto" (tramite `DiscoveryCache`) e poi inoltra la richiesta bloccandosi asincronamente in attesa della risposta.
   - Include logiche di timeout configurabili (`PacomError::DiscoveryTimeout`).

## Design Pattern: "Closure Adapters"
L'interfaccia asincrona standard di `up-rust` richiede di implementare struct che aderiscano al trait `UListener`. Per non appesantire il codice utente (che implicherebbe lifetime complessi e Arc ovunque), questa API utilizza pattern "Adapter", racchiudendo le function `impl Fn(...)` in struct interne (`ClosureListener`, `ClosureHandler`) e sbrigando internamente il binding con il runtime Tokio.
