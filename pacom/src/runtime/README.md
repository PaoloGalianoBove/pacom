# Modulo Runtime (`runtime/`)

Il modulo `runtime` contiene il comportamento operativo "nascosto" di PACOM: validazione del manifesto, traduzione degli indirizzi, meccanismi di Service Discovery, e orchestrazione dei trasporti.

## `engine.rs` (Il Cuore dell'Orchestrazione)

Il file `engine.rs` espone la struct `RuntimeEngine` (solitamente mascherata da `PacomRuntime` al livello superiore).

### Funzioni e Strutture di Rilievo:
- **`DiscoveryCache`**: Una struttura dati thread-safe (`RwLock`) che memorizza tutti i nodi scoperti (RPC Providers e Topic Publishers) associandone il nome logico al loro `UUri` numerico reale.
- **`DiscoveryListener`**: Un `UListener` specializzato che resta in ascolto sui canali di uDiscovery configurati. Quando riceve un payload di discovery (attualmente serializzato in JSON), aggiorna la `DiscoveryCache`.
- **`PendingSubscription`**: Se l'utente prova a sottoscrivere un topic il cui publisher non è ancora noto, PACOM non fallisce immediatamente ma inserisce la richiesta in pending. Quando il Local Discovery intercetta il publisher, la subscription viene attivata sul router.

## `logical_registry.rs` (La Tabella di Routing)

Mappa i nomi logici leggibili in ID uProtocol numerici (`u16`) usando un algoritmo di hashing veloce (FNV-1a) a 16 bit. Il modulo mantiene una risoluzione deterministica degli ID per le capability dichiarate nel manifest e, per gli elementi pubblicati o forniti, evita conflitti locali tramite riallocazione incrementale nello stesso semispazio.

### Funzioni di Calcolo (Il "Split" degli ID):
Per evitare che l'hash di un topic collida inavvertitamente con l'hash di un RPC:
- **`method_id_for(logical_method)`**: Forza il bit più significativo a `0` (Range: `0x0001..=0x7FFF`).
- **`resource_id_for(logical_topic)`**: Forza il bit più significativo a `1` (Range: `0x8000..=0xFFFF`).

Nota: la validazione `validate_no_collisions()` e mantenuta per compatibilità con il flusso di bootstrap, ma la gestione effettiva degli ID avviene in `resolve_and_store_ids()`.

## Il Flusso della Discovery

Lo standard uProtocol impone che la discovery avvenga sul servizio `0x0F00`. In PACOM, per default si usano 16 istanze (`0x0F00` a `0x0F0F`), con cardinalità configurabile tramite `PACOM_DISCOVERY_CHANNELS`.
All'avvio, `RuntimeEngine`:
1. Verifica se l'app consuma/legge dati (dalla configurazione del manifest).
2. Se sì, fa un `register_listener` massivo sui **16 canali** di discovery per intercettare gli annunci di chiunque.
3. Inoltre, lancia un task asincrono in background che ogni `PACOM_DISCOVERY_REANNOUNCE_SECS` riannuncia le capability offerte dall'app, in modo che eventuali listener avviati in ritardo possano allinearsi.
