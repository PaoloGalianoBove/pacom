# Modulo transport

Il modulo transport implementa i bridge di rete e il routing tra percorsi locali e cloud.

## vsomeip.rs

Responsabilita principali:
1. setup dinamico configurazione vSomeIP
2. role election router/client su host condiviso
3. gestione lock/socket stale e bootstrap robusto in container

Variabili utili:
- `PACOM_VSOMEIP_CONFIG_PATH`
- `VSOMEIP_CONFIGURATION`
- `PACOM_VSOMEIP_ROLE`
- `PACOM_VSOMEIP_LOCK_STALE_MS`
- `PACOM_VSOMEIP_ELECTION_WAIT_MS`
- `PACOM_DEBUG_VERBOSE`

## mqtt.rs

Gestisce il trasporto off-vehicle via MQTT 5. E usato in combinazione col router per traffico cloud-bound.

## router.rs

PacomRouter implementa `UTransport` e decide il percorso:
- locale vSomeIP quando l'URI non e cloud-bound
- MQTT quando il target e marcato cloud-bound (es. marker cross-domain o wildcard authority-level)

Dettagli importanti:
- normalizza alcuni source filter locali per evitare registrazioni SOME/IP wildcard ambigue
- evita candidate wildcard locali per ridurre conflitti di service offer
- mantiene log diagnostici dettagliati quando `PACOM_DEBUG_VERBOSE=true`

## Invarianti da preservare

- Non riscrivere topic publish locale in notification.
- Non introdurre mapping case-specific hardcoded per topic singoli.
- Mantenere coerenza tra routing decision e registrazione listener.

## Contribuire in sicurezza

- isolare sempre i test vSomeIP in processi/container separati
- evitare cambiamenti distruttivi nel lifecycle FFI
- verificare ogni modifica con almeno un test locale e uno cloud path
