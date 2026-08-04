# Modulo Public API (`public_api/`)

Questo modulo costituisce il punto d'ingresso principale (entrypoint) per gli sviluppatori di applicazioni. Nasconde la complessità della logica asincrona di uProtocol dietro interfacce user-friendly e tipizzate.

## Componenti Principali

### `PacomRuntime`
L'oggetto centrale del sistema (spesso racchiuso in un `Arc<PacomRuntime>` nei thread applicativi). Inizializza l'orchestrazione locale e funge da facade verso runtime e trasporti.

#### Funzioni Esposte:

1. **`new(config: RuntimeConfig) -> Result<Self, PacomError>`**
   - Inizializza il runtime. Esegue il parsing di `manifest.json`, imposta la `DiscoveryCache`, istanzia il `PacomRouter` e avvia i trasporti `vSomeIP` e `MQTT` se richiesti dalla configurazione.
   - Se `manifest_path` non è fornito, usa `PACOM_MANIFEST_PATH` oppure `/etc/pacom/manifest.json`.

2. **`subscribe_event(&self, topic: &str, callback: impl Fn(Vec<u8>))`**
   - Registra una closure da eseguire ogni volta che un evento viene ricevuto sul topic logico specificato.
   - Nasconde l'uso del trait `UListener` instanziando internamente un `ClosureListener`.
   - Se il publisher non e ancora stato scoperto, la subscribe resta pending e viene attivata quando arriva un annuncio di discovery.

3. **`publish_event(&self, topic: &str, payload: Vec<u8>)`**
   - Invia un payload binario al topic logico specificato.
   - Per topic locali genera un messaggio `Publish`; per topic cloud delega il routing al `PacomRouter` usando la cloud authority configurata a runtime.

4. **`register_rpc_method(&self, method: &str, handler: impl Fn(Vec<u8>) -> Future)`**
   - Registra un servizio di Request/Response. La closure riceve il payload della richiesta e deve restituire una `Future` che risolve nel payload di risposta.
   - Si aggancia al `RequestHandler` standard di `up-rust`.

5. **`invoke_rpc_method(&self, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, PacomError>`**
   - Chiama un servizio RPC. Questa funzione attende prima che il servizio sia stato "scoperto" (tramite `DiscoveryCache`) e poi inoltra la richiesta bloccandosi asincronamente in attesa della risposta.
   - Include logiche di timeout configurabili (`PacomError::DiscoveryTimeout`).

## Design Pattern: "Closure Adapters"
L'interfaccia asincrona standard di `up-rust` richiede di implementare struct che aderiscano a trait come `UListener` e `RequestHandler`. Per non appesantire il codice utente, questa API utilizza adapter interni (`ClosureListener`, `ClosureHandler`) che racchiudono le closure e le collegano al runtime.
