# Modulo runtime

Il modulo runtime contiene il comportamento operativo di PACOM: validazione manifest, discovery, publish/subscribe e RPC su trasporti multipli.

## engine.rs in breve

RuntimeEngine mantiene:
- router trasporti (vSomeIP + MQTT)
- client/server RPC di up-rust
- cache discovery concorrente
- capability locali pubblicate (RPC provide + topic publish)
- pending subscriptions per topic non ancora scoperti

## Discovery attuale

PACOM usa un canale discovery interno su URI fissa `0x0F00/0x8F01` (16 canali, selezione per `UP_UE_ID % 16`).

Flusso:
1. ogni provider annuncia `rpc_provide` e `topic_publish` con payload JSON
2. i consumer aggiornano una cache locale provider per nome logico
3. se una subscribe topic era in pending, viene attivata al primo announce
4. un task periodico (`PACOM_DISCOVERY_REANNOUNCE_SECS`) riannuncia le capability

Nota importante: per i topic publish, il provider UE-ID annunciato deve coincidere con l'UE-ID realmente usato nel publish.

## Split UE-ID tra RPC e topic publish

Per evitare collisioni SOME/IP tra offer RPC e publish topic, il runtime usa:
- UE-ID applicativo base per RPC
- UE-ID derivato per topic publish (`derive_topic_publish_ue_id`)

La stessa identità UE topic viene usata sia nel messaggio publish sia negli annunci discovery (iniziali e periodici).

## Startup order e semantica eventi

La discovery rende robusta la registrazione listener anche se il subscriber parte dopo.
Gli eventi publish restano fire-and-forget: se un subscriber non era attivo nel momento di un evento, quell'evento non viene replayato automaticamente.

## logical_registry.rs

Mappa nomi logici stringa in ID uProtocol numerici con FNV-1a 16 bit e controlla collisioni a startup:
- range metodi RPC: `0x0001..=0x7FFF`
- range topic/eventi: `0x8000..=0xFFFF`

In caso di collisione il boot viene bloccato con errore esplicito.

## Variabili utili

- `UP_AUTHORITY`
- `UP_UE_ID`
- `PACOM_DISCOVERY_MAX_WAIT_MS`
- `PACOM_DISCOVERY_POLL_MS`
- `PACOM_DISCOVERY_REANNOUNCE_SECS`
- `PACOM_DEBUG_VERBOSE`
- `PACOM_ENABLE_LOCAL_WILDCARD_SUBSCRIBE`

## Linee guida contributo

- evitare panic/unwrap nel path runtime
- mantenere coerenza tra identita discovery e identita traffico reale
- usare log verbose solo sotto `PACOM_DEBUG_VERBOSE`
- preferire modifiche generali (non case-specific)
