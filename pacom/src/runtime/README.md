# Modulo Runtime (`runtime/`)

Il modulo `runtime` contiene il comportamento operativo "nascosto" di PACOM: validazione del manifesto, traduzione degli indirizzi, meccanismi di Service Discovery, e orchestrazione dei trasporti.

## `engine.rs` (Il Cuore dell'Orchestrazione)

Il file `engine.rs` espone la struct `RuntimeEngine` (solitamente mascherata da `PacomRuntime` al livello superiore).

### Funzioni e Strutture di Rilievo:
- **`DiscoveryCache`**: Una struttura dati thread-safe (`RwLock`) che memorizza tutti i nodi scoperti (RPC Providers e Topic Publishers) associandone il nome logico al loro `UUri` numerico reale.
- **`DiscoveryListener`**: Un `UListener` specializzato che resta perennemente in ascolto sui 16 canali di uDiscovery. Quando riceve un payload (attualmente formattato in JSON per comodità rapida), aggiorna la `DiscoveryCache`.
- **`PendingSubscription`**: Qualora l'utente provi a sottoscrivere un topic che non è ancora stato offerto sulla rete da nessuno, PACOM non va in errore ma inserisce la richiesta in pending. Non appena il Local Discovery intercetta il publisher, la subscription viene finalizzata automaticamente sul Router.

## `logical_registry.rs` (La Tabella di Routing)

Mappa i nomi logici leggibili in ID uProtocol numerici (`u16`) usando un algoritmo di hashing veloce (FNV-1a) a 16 bit. Include forti controlli anti-collisione eseguiti al boot.

### Funzioni di Calcolo (Il "Split" degli ID):
Per evitare che l'hash di un topic collida inavvertitamente con l'hash di un RPC:
- **`method_id_for(logical_method)`**: Forza il bit più significativo a `0` (Range: `0x0001..=0x7FFF`).
- **`resource_id_for(logical_topic)`**: Forza il bit più significativo a `1` (Range: `0x8000..=0xFFFF`).

## Il Flusso della Discovery

Lo standard uProtocol impone che la Discovery avvenga sul servizio `0x0F00`. Dato l'elevato numero di app veicolari, si usano 16 istanze per bilanciare il carico (`0x0F00` a `0x0F0F`).
All'avvio, `RuntimeEngine`:
1. Verifica se l'app consuma/legge dati (dalla configurazione del manifest).
2. Se sì, fa un `register_listener` massivo sui **16 canali** di discovery per intercettare gli annunci di chiunque.
3. Inoltre, lancia un task asincrono in background che ogni `PACOM_DISCOVERY_REANNOUNCE_SECS` "urla" sulla rete quali servizi l'app stessa sta fornendo, in modo che eventuali listener avviati in ritardo possano allinearsi.
