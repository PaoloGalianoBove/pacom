# Modulo `transport`

Il modulo `transport` contiene il livello più basso del framework: i bridge verso la rete fisica (Layer 1 di uProtocol). Qualsiasi file qui dentro deve implementare il trait `UTransport` fornito dalla libreria `up-rust`.

## `vsomeip.rs` (Comunicazione In-Vehicle)

Questo è il trasporto principale di PACOM. Si appoggia alla libreria open source `up-transport-vsomeip`, che a sua volta è un wrapper Rust tramite FFI della famosa libreria C++ **vSomeIP** del consorzio COVESA (standard de facto per l'Automotive Ethernet).

### Compiti di `vsomeip.rs`
1. **Configurazione Automatica**: vSomeIP è un demone capriccioso che richiede un file di configurazione JSON obbligatorio per partire. Il nostro `vsomeip.rs` tenta di semplificare la vita allo sviluppatore: se non viene fornita alcuna configurazione esterna, genera al volo un file in `/tmp/vsomeip-*.json` leggendo il nome dell'applicazione dall'ambiente (`UP_UE_ID`) e ricavando l'indirizzo IP locale della macchina.
2. **Ciclo di Vita Thread**: Poiché vSomeIP è C++, una volta chiamato il comando di start, questo bloccherebbe l'intero programma Rust. Pertanto `vsomeip.rs` spinge l'avvio della libreria C++ all'interno di un thread separato (usando `std::thread::spawn`) che rimane in background.
3. **Traduzione dei payload**: Prende il payload in byte, costruisce una request vSomeIP standard, e viceversa quando riceve pacchetti dal socket di rete inverte il processo chiamando i `UListener` di Rust.

**Attenzione per i contributor**: 
vSomeIP **non supporta nativamente** l'esistenza di più "applicazioni" nello stesso processo OS. Se tenti di creare due istanze di trasporto vSomeIP nello stesso programma (es. in un test unitario), i distruttori andranno in panic (SIGABRT). I test che usano questo modulo devono essere eseguiti in processi isolati.

## `mqtt.rs` (Comunicazione Off-Board / Cloud)

L'auto moderna non comunica solo internamente, ma anche col cloud. Questo trasporto usa la libreria `up-transport-mqtt5`.
1. Si connette a un broker MQTT (es. Mosquitto) leggendo l'URI dalla variabile `PACOM_MQTT_BROKER_URI`.
2. I messaggi uProtocol vengono inseriti in pacchetti MQTT formali, in cui il topic MQTT è generato serializzando l'`UUri` (secondo le specifiche uProtocol per MQTT).
3. Non usa vSomeIP, quindi non subisce le limitazioni di multicast o multi-processo.

## `router.rs` (uStreamer)

Questo è il semaforo intelligente del middleware. Implementa il trait `UTransport`, ma non invia pacchetti direttamente sulla rete: fa da proxy.

uProtocol permette di creare un **uStreamer**, un nodo software che collega due mondi. Il nostro `router.rs` viene inizializzato passandogli i reference di `vsomeip` e `mqtt`.

Quando tu chiami `runtime.publish()`, il pacchetto arriva prima al `router.rs`. 
1. Il router legge la destinazione (l'`authority` contenuta nell'`UUri`).
2. Se l'authority corrisponde all'authority locale dell'ECU (`UP_AUTHORITY`), instrada il messaggio al trasporto **`vsomeip`**.
3. Se l'authority è destinata al cloud o ad un'altra ECU irraggiungibile via SOME/IP, instrada il messaggio verso il trasporto **`mqtt`**.

Questa architettura permette di sviluppare Gateway veicolari in maniera trasparente. Un'app di sensori può spedire il dato a un'altra app, e senza dover cambiare il codice, se si cambia l'authority di destinazione il messaggio magicamente volerà via MQTT verso il server remoto.

## Come contribuire a `transport`

- **Aggiungere un nuovo protocollo (es. Zenoh, DDS, o HTTP)**: Crea un nuovo file `zenoh.rs`. Crea uno struct che implementi il trait `UTransport`. Dopodiché, vai in `router.rs` e aggiungi il tuo nuovo trasporto come opzione di routing.
- **Risoluzione Bug vSomeIP**: Essendo un wrapper FFI (Foreign Function Interface), se vedi segmentation fault (`SIGSEGV`), il problema risiede al 99% nel lifecycle di `vsomeip.rs` (puntatori rilasciati troppo presto o inizializzazioni mancanti). Maneggia con cautela l'ordine di shutdown del thread C++.
