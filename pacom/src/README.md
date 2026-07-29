# PACOM: Source Map e Linee Guida

Questo documento funge da mappa dettagliata per orientarsi nel codice sorgente (`src/`) e comprendere le responsabilità di ogni modulo.

## 1. `public_api/` (Interfaccia Utente)
Questa cartella contiene il wrapper asincrono destinato all'utente finale. Tutte le funzioni pubbliche espongono una firma idiomatica in puro Rust.
- **`PacomRuntime`**: L'oggetto thread-safe (`Arc`-based) che rappresenta il nodo. 
- Espone funzioni come `subscribe_event`, `publish_event`, e `register_rpc_method`.
- *Design Decision*: L'uso di normali closure `Fn(Vec<u8>)` maschera completamente i complessi trait asincroni richiesti da `up-rust` (es. `UListener`), rendendo il codice dell'app finale compatto e leggibile.

## 2. `runtime/` (Il Motore Centrale)
Il cuore operativo che instrada le chiamate e risolve gli identificativi.
- **`engine.rs (RuntimeEngine)`**: Contiene la logica per allocare il Router, istanziare i Client/Server RPC e gestire il polling del Discovery.
- **`logical_registry.rs (LogicalRegistry)`**: Effettua il parsing del file `manifest.json`.
  - *Design Decision (Lo Split degli ID)*: Siccome uProtocol mappa le risorse su 16-bit (`0x0000` a `0xFFFF`), la funzione di calcolo dell'hash divide brutalmente lo spazio per evitare collisioni: la prima metà (`0x0001` - `0x7FFF`) è strettamente riservata agli **RPC**, mentre la seconda (`0x8000` - `0xFFFF`) è riservata ai **Topic**.

## 3. `transport/` (I Bridge di Rete)
Qui vengono materializzati i socket verso l'infrastruttura veicolare e cloud.
- **`vsomeip.rs`**: Effettua il setup dinamico di vSomeIP e la negoziazione dei ruoli sul nodo.
- **`mqtt.rs`**: Configura la connessione persistente al broker MQTT5 per i messaggi off-vehicle.
- **`router.rs (PacomRouter)`**: Un oggetto che implementa l'interfaccia standard `UTransport`. Per ogni messaggio calcola la destinazione tramite `is_cloud_bound` e instrada il payload.
- **`vsomeip_topology.rs (VsomeipTopologyResolver)`**: Una struttura matematica che protegge il router dalle rigidità di vSomeIP. Poiché vSomeIP andrebbe in crash ricevendo una wildcard (es. `*`), questo modulo converte le iscrizioni wildcard in un array deduplicato di istanze IP concrete.

## Gestione Errori (`error.rs`)
Ogni fallimento interno (timeout di discovery, collisioni ID, configurazioni errate) viene intercettato usando la libreria `thiserror` (struttura `PacomError`). 
Il modulo provvede poi a convertire automaticamente l'errore Rust nativo in un codice `UCode` (es. `DEADLINE_EXCEEDED` o `PERMISSION_DENIED`) compatibile col protocollo uProtocol, garantendo messaggi d'errore puliti nei log.

## Regole Operative Importanti
- **Trasparenza del Topic**: Non cambiare mai la semantica di un Publish locale.
- **Log Gated**: Evita log rumorosi in produzione. Qualsiasi diagnostica complessa (es. print di pacchetti hex) deve essere condizionata alla variabile `PACOM_DEBUG_VERBOSE=true`.
