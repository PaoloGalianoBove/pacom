# Modulo Transport (`transport/`)

Il modulo `transport` contiene l'implementazione fisica della rete. Si occupa di inizializzare le librerie di base (vSomeIP e MQTT) e instradarvi i messaggi (UTransport routing).

## 1. `router.rs` (`PacomRouter`)
Il cuore dell'instradamento di rete. Implementa il trait `UTransport` richiesto dallo standard `up-rust`.

### Funzionamento Principale:
- Riceve ogni singolo pacchetto (UMessage) inviato dall'applicazione.
- **`is_cloud_bound(uri)`**: Metodo fondamentale. Controlla l'authority del pacchetto. Se il campo "Authority" è vuoto o ha un asterisco `*`, significa che il pacchetto è locale (intra-vehicle) e va inoltrato al driver vSomeIP. Se invece presenta un'authority marcata (es. una stringa FQDN), il pacchetto è diretto al cloud e va inoltrato al driver MQTT5.
- Svolge il delicatissimo compito di coordinare il `register_listener` tra due mondi concettualmente diversi (Pub/Sub via MQTT contro Servizi/Istanze via vSomeIP).

## 2. `vsomeip_topology.rs` (`VsomeipTopologyResolver`)
Una delle maggiori complessità di uProtocol applicato all'Automotive risiede nell'incompatibilità intrinseca tra la specifica uProtocol (che ammette wildcard come l'asterisco `*` per sottoscriversi a "qualsiasi" entità) e la specifica SOME/IP (che non ammette iscrizioni generiche ma solo IP/ID concreti).

Questo modulo è incaricato di sporcarsi le mani:
- **`expand_listener_candidates(...)`**: Prende una stringa uProtocol contenente wildcard ed espande una serie di array `Vec<UUri>` contenenti tutte le possibili istanze concrete (es. Authority vuota, Authority locale). Successivamente, **deduplica** l'array.
In questo modo, il Router non si troverà mai a registrare due volte la stessa identità vSomeIP (che causerebbe errori a runtime come istanze "0xFF" contrastanti).

## 3. `vsomeip.rs` e `mqtt.rs` (I Driver)
Questi file contengono una logica di interfacciamento quasi nativa per i rispettivi protocolli, gestendo i `Mutex` e i thread asincroni per garantire che le code di ricezione MQTT e i thread di polling C++ di vSomeIP (tramite `up-transport-vsomeip`) girino fluidamente e non blocchino Tokio.

- In vSomeIP viene eseguita la `role election`: in assenza di un master, il nodo diventerà il master vSomeIP locale.
- In MQTT, la connessione è governata dal file di configurazione e viene agganciata agli argomenti forniti in `RuntimeConfig`.
