# PACOM source map

Questo documento riassume come e dove intervenire nel codice sotto `src/`.

## Layer principali

1. `public_api/`
   API ad alto livello per applicazioni (`publish`, `subscribe`, `invoke_method`, `register_rpc_method`).

2. `runtime/`
   Motore operativo: validazione manifest, mapping logico->ID, discovery, pending subscriptions e RPC orchestration.

3. `transport/`
   Bridge fisici:
   - `vsomeip.rs` per traffico locale
   - `mqtt.rs` per traffico cloud
   - `router.rs` per routing decision tra i due

## Regole operative importanti

- Non cambiare la semantica dei topic locali: topic publish resta publish.
- Mantieni coerenza tra discovery provider UE-ID e sorgente reale dei messaggi.
- Per cloud command/event usa le API mirate con authority (`publish_to_authority`, `subscribe_from_authority`).
- Evita log rumorosi non gated: i log diagnostici devono passare da `PACOM_DEBUG_VERBOSE`.

## Startup behavior da ricordare

- La discovery rende robusto l'attach dei subscriber anche con ordine di startup invertito.
- Gli eventi publish sono live: niente replay automatico se un consumer era offline.
- Il primissimo comando cloud puo essere perso se inviato prima che il provider abbia registrato la subscribe cloud.

## Dove cercare in debug

- Issue topic/discovery: `runtime/engine.rs`
- Routing cloud/local: `transport/router.rs`
- Startup vSomeIP e role election: `transport/vsomeip.rs`
- Errori contratto manifest: `runtime/logical_registry.rs` e `error.rs`
