# Modulo Transport (`transport/`)

Il modulo `transport` contiene l'implementazione fisica della rete. Si occupa di inizializzare le librerie di base (vSomeIP e MQTT) e instradarvi i messaggi (UTransport routing).

## 1. `router.rs` (`PacomRouter`)
Il cuore dell'instradamento di rete. Implementa il trait `UTransport` richiesto dallo standard `up-rust`.

### Funzionamento Principale:
- Riceve ogni singolo pacchetto (UMessage) inviato dall'applicazione.
- **`is_cloud_bound(uri)`**: Metodo fondamentale. Considera locale il traffico con authority vuota o wildcard. Considera cloud-bound il traffico che usa il marker cross-domain (`ue_id=0`, `major=0`, `resource_id=0`), le wildcard MQTT o i casi in cui vSomeIP è disabilitato. Una authority valorizzata da sola non basta sempre a classificare il traffico come cloud.
- Coordina il `register_listener` tra due mondi concettualmente diversi (Pub/Sub via MQTT contro Servizi/Istanze via vSomeIP).

## 2. `vsomeip_topology.rs` (`VsomeipTopologyResolver`)
Una delle maggiori complessità di uProtocol applicato all'Automotive risiede nell'incompatibilità intrinseca tra la specifica uProtocol (che ammette wildcard come l'asterisco `*` per sottoscriversi a "qualsiasi" entità) e la specifica SOME/IP (che non ammette iscrizioni generiche ma solo IP/ID concreti).

Questo modulo è incaricato di sporcarsi le mani:
- **`expand_listener_candidates(...)`**: Prende una stringa uProtocol contenente wildcard ed espande una serie di array `Vec<UUri>` contenenti tutte le possibili istanze concrete (es. Authority vuota, Authority locale). Successivamente, **deduplica** l'array.
In questo modo, il router evita registrazioni duplicate verso vSomeIP e non propaga wildcard non supportate al driver locale.

## 3. `vsomeip.rs` e `mqtt.rs` (I Driver)
Questi file contengono la logica di interfacciamento verso i rispettivi protocolli, gestendo sincronizzazione e task asincroni in modo da integrare il polling MQTT e i thread C++ di vSomeIP (tramite `up-transport-vsomeip`) con il runtime Tokio.

- In vSomeIP viene eseguita la `role election`: in assenza di un master, il nodo diventerà il master vSomeIP locale.
- In MQTT, la connessione è governata dai parametri `broker_uri` e `client_id` forniti in `RuntimeConfig`.
