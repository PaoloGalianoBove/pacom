# PACOM: Source Map e Linee Guida

Questo documento funge da mappa dettagliata per orientarsi nel codice sorgente (`src/`) e comprendere le responsabilità di ogni modulo.

## 1. `public_api/` (Interfaccia Utente)
Questa cartella contiene il wrapper asincrono destinato all'utente finale. Tutte le funzioni pubbliche espongono una firma idiomatica in puro Rust.
- **`PacomRuntime`**: Il facade principale che rappresenta il nodo applicativo. Nei programmi utente viene spesso racchiuso in `Arc`, ma il tipo non impone questo dettaglio.
- Espone funzioni come `subscribe_event`, `publish_event`, `invoke_rpc_method` e `register_rpc_method`.
- *Design Decision*: L'uso di normali closure `Fn(Vec<u8>)` maschera completamente i complessi trait asincroni richiesti da `up-rust` (es. `UListener`), rendendo il codice dell'app finale compatto e leggibile.

## 2. `runtime/` (Il Motore Centrale)
Il cuore operativo che instrada le chiamate e risolve gli identificativi.
- **`engine.rs (RuntimeEngine)`**: Contiene la logica per allocare il Router, istanziare i Client/Server RPC e gestire il polling del Discovery.
- **`logical_registry.rs (ManifestConfig)`**: Effettua il parsing del file `manifest.json` e risolve i nomi logici nei relativi ID.
  - *Design Decision (Lo Split degli ID)*: Siccome uProtocol mappa le risorse su 16-bit (`0x0000` a `0xFFFF`), la funzione di calcolo dell'hash separa lo spazio: la prima metà (`0x0001` - `0x7FFF`) è riservata agli **RPC**, mentre la seconda (`0x8000` - `0xFFFF`) è riservata ai **Topic**.
  - Le collisioni tra capability dichiarate in `rpc.provide` e `topics.publish` vengono gestite localmente durante la risoluzione degli ID, senza richiedere configurazioni statiche allo sviluppatore.

## 3. `transport/` (I Bridge di Rete)
Qui vengono materializzati i socket verso l'infrastruttura veicolare e cloud.
- **`vsomeip.rs`**: Effettua il setup dinamico di vSomeIP e la negoziazione dei ruoli sul nodo.
- **`mqtt.rs`**: Configura la connessione persistente al broker MQTT5 per i messaggi off-vehicle.
- **`router.rs (PacomRouter)`**: Un oggetto che implementa l'interfaccia standard `UTransport`. Per ogni messaggio calcola la destinazione tramite `is_cloud_bound` e instrada il payload.
- **`vsomeip_topology.rs (VsomeipTopologyResolver)`**: Un modulo di adattamento che protegge il router dalle rigidità di vSomeIP. Poiché vSomeIP non gestisce direttamente le wildcard nello stesso modo di uProtocol, questo modulo converte le iscrizioni wildcard in un insieme deduplicato di istanze concrete.

## Gestione Errori (`error.rs`)
Ogni fallimento interno (timeout di discovery, collisioni ID, configurazioni errate) viene intercettato usando la libreria `thiserror` (struttura `PacomError`).
Il modulo provvede poi a convertire automaticamente l'errore Rust nativo in un codice `UCode` (es. `DEADLINE_EXCEEDED` o `PERMISSION_DENIED`) compatibile col protocollo uProtocol, mantenendo una semantica degli errori coerente tra runtime e trasporto.

## Regole Operative Importanti
- **Trasparenza del Topic**: La semantica del publish locale va preservata anche quando il runtime introduce adattamenti per il trasporto sottostante.
- **Log Gated**: Evita log rumorosi in produzione. Qualsiasi diagnostica complessa (es. print di pacchetti hex) deve essere condizionata alla variabile `PACOM_DEBUG_VERBOSE=true`.
