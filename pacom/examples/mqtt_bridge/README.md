# Esempio mqtt_bridge

Esempio completo con tre app:
1. `light_dashboard` (HMI locale)
2. `light_switch` (controller locale + bridge cloud)
3. `cloud_app` (client cloud MQTT)

## Flussi principali

- Dashboard -> Switch: RPC su `/rpc/lights/set`
- Switch -> Dashboard: topic `/status/lights`
- Cloud app -> Switch: topic `/cloud/command` con authority target `ecu-switch`
- Switch -> Cloud app: topic `/cloud/telemetry`

## Note operative importanti

- Lo stato topic locale usa publish standard (non notification).
- Dashboard puo partire prima o dopo switch: la subscribe viene agganciata via discovery reannounce.
- Gli eventi sono live: se un consumer non era attivo, non c'e replay automatico del vecchio evento.
- Il primo comando cloud puo essere perso se inviato troppo presto (race di startup). Attendere bootstrap o implementare gating ready lato cloud app.

## Esecuzione rapida

Avvia i tre container con i README di deploy in questa cartella.

## Verifica minima

1. invia comando dal dashboard e verifica update locale
2. invia comando dal cloud app e verifica update su switch e dashboard
3. verifica telemetria cloud
