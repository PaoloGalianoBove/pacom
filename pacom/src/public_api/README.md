# Modulo public_api

Questo modulo espone la facciata ad alto livello di PACOM. Le app dovrebbero usare solo questa API.

## Oggetto principale

`PacomRuntime` incapsula `RuntimeEngine` e offre metodi logici su stringhe, non su URI numerici.

## Metodi esposti

Publish/Subscribe:
- `publish(topic, payload)`
- `subscribe(topic, callback)`
- `publish_event(...)` alias di `publish`
- `subscribe_event(...)` alias di `subscribe`
- `publish_event_to(topic, authority, payload)` per cross-domain
- `subscribe_event_from(topic, authority, callback)` per cross-domain

RPC:
- `register_rpc_method(service, handler)`
- `call_rpc(service, payload)`
- `invoke_method(...)` alias di `call_rpc`

## Contratti importanti

- ogni metodo valida il manifest (`rpc.provide`, `rpc.consume`, `topics.publish`, `topics.subscribe`)
- gli errori applicativi vengono normalizzati in `PacomError`
- la logica di routing/discovery resta nel runtime e non deve salire in public_api

## Linee guida contributo

- mantenere firme semplici e stabili
- evitare dipendenze da tipi interni uProtocol nella superficie pubblica
- spostare logica complessa in `runtime`
