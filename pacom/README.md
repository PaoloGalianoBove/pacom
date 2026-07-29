# PACOM: Protocol Agnostic Communication Manager

PACOM è un layer di orchestrazione e comunicazione costruito sopra le specifiche di [uProtocol](https://github.com/eclipse-uprotocol). Agisce come "colla" architetturale tra le logiche applicative ad alto livello e i protocolli di trasporto sottostanti, nascondendo la complessità della rete e permettendo agli sviluppatori di concentrarsi sulla business logic.

## Cos'è PACOM?

In uno scenario automotive moderno, le centraline (ECU) locali comunicano tramite protocolli intra-veicolo (come **vSomeIP**), mentre la comunicazione verso l'infrastruttura Cloud avviene tramite protocolli cross-domain (come **MQTT**). 
PACOM risolve l'attrito (impedance mismatch) tra questi due mondi, gestendo il routing dei messaggi in modo del tutto trasparente. Le applicazioni non devono preoccuparsi di quali trasporti siano attivi o di come tradurre gli ID.

### Caratteristiche Chiave
1. **Configurazione Dichiarativa**: Tutto parte dal file `manifest.json`. Si dichiarano le stringhe logiche degli RPC e dei Topic, e PACOM calcola in automatico le traduzioni ID sicure a 16-bit.
2. **Astrazione del Trasporto**: PACOM capisce automaticamente se inviare un messaggio sul bus vSomeIP locale o al Broker MQTT nel cloud.
3. **Local Discovery**: Registrazione dinamica su 16 canali. Le app locali si scoprono a vicenda e si scambiano le capacità offerte, a prescindere dall'ordine di avvio.

---

## Architettura e Layer del Codice

Il codice in `src/` è strutturato a cipolla. Ogni directory contiene un proprio `README.md` con l'analisi delle singole funzioni. Ecco una mappa ad alto livello:

### 1. `public_api/` (Interfaccia Utente)
Espone l'oggetto principale `PacomRuntime`, che è l'unico punto di accesso per l'applicazione finale.
- **`PacomRuntime::new(...)`**: Inizializza il motore caricando il manifesto, e avviando le connessioni a vSomeIP e MQTT se richieste.
- **`publish_event(...)` / `subscribe_event(...)`**: Metodi per inviare o ricevere messaggi Pub/Sub su topic logici locali.
- **`invoke_method(...)` / `register_rpc_method(...)`**: Metodi per chiamare o esporre servizi Request/Response.

### 2. `runtime/` (Il Cervello)
Qui avviene la magia della traduzione e della scoperta.
- **`RuntimeEngine`**: La struttura nascosta dietro a `PacomRuntime`. Gestisce la cache della discovery, smista le callback e mantiene vivi i client/server RPC in memoria.
- **`LogicalRegistry`**: Legge il `manifest.json` ed effettua l'hashing stringa -> intero a 16 bit, dividendo rigorosamente lo spazio ID a metà (RPC vs Topic) per evitare collisioni matematiche.

### 3. `transport/` (I Muscoli)
Qui i pacchetti vengono materialmente immessi sulla rete.
- **`PacomRouter`**: Implementa il trait `UTransport`. Analizza la destinazione (`is_cloud_bound`) di ogni pacchetto e lo invia al socket giusto.
- **`VsomeipTopologyResolver`**: Poiché vSomeIP non tollera wildcard (asterischi) a differenza di uProtocol, questa struct maschera la rigidità di vSomeIP pre-calcolando e deduplicando le rotte consentite.

---

## Quickstart (Esempio d'Uso)

Un esempio minimale di inizializzazione e iscrizione a un topic:

```rust
use pacom::{PacomRuntime, RuntimeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Inizializza il runtime (legge manifest.json di default)
    let runtime = PacomRuntime::new(RuntimeConfig::default()).await?;

    // 2. Iscrizione a un evento (Topic Logico)
    runtime.subscribe_event("/status/lights", |payload| {
        let msg = String::from_utf8_lossy(&payload);
        println!("Stato luci ricevuto: {}", msg);
    }).await?;

    // 3. Pubblica un evento
    runtime.publish_event("/status/lights", b"ON".to_vec()).await?;

    // Tieni in vita il processo
    std::future::pending::<()>().await;
}
```

## Esecuzione e Demo (Docker)

Il progetto include degli esempi pronti all'uso in `examples/mqtt_bridge/deploy`. Troverai un file Docker Compose che avvia:
- **Broker MQTT**: Simulatore Cloud.
- **Light Switch**: Nodo vSomeIP.
- **Cloud App**: Simula un'app Cloud connessa a MQTT.

```bash
cd examples/mqtt_bridge/deploy
docker compose up --build
```

## Build Locale
Assicurati di avere `vsomeip3` installato sul tuo sistema:
```bash
cargo build --release
```
