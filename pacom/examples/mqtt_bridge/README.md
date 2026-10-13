# Esempio mqtt_bridge

Esempio completo con tre app:
1. `light_dashboard` (HMI locale)
2. `light_switch` (controller locale + bridge cloud)
3. `cloud_app` (client cloud MQTT)

## Flussi principali

- Dashboard -> Switch: RPC su `/rpc/lights/set`
- Switch -> Dashboard: topic `/status/lights`
- Cloud app -> Switch: topic `/cloud/command` instradato sul dominio cloud tramite l'authority configurata in `PACOM_CLOUD_AUTHORITY`
- Switch -> Cloud app: topic `/cloud/telemetry`

## Note operative importanti

- Lo stato topic locale usa publish standard (non notification).
- Dashboard puo partire prima o dopo switch: la subscribe viene agganciata via discovery reannounce.
- Gli eventi sono live: se un consumer non era attivo, non c'e replay automatico del vecchio evento.
- Il primo comando cloud puo essere perso se inviato troppo presto (race di startup). Attendere bootstrap o implementare gating ready lato cloud app.

## Esecuzione rapida

Avvia i tre container con i README di deploy in questa cartella.

Nel menu della dashboard o del client cloud, l'opzione `2` chiede un colore RGB
come sei cifre esadecimali senza `#` (ad esempio `12A4FF`) e lo invia allo
switch. L'opzione `0` invia `#000000` per spegnere il colore. Lo switch attua il
segnale KUKSA `Vehicle.Cabin.Light.AmbientLight.Row1.DriverSide.Color`, mappato
dal provider CAN al segnale DBC `Light_RGB`.

I comandi high/low beam restano nel codice commentati, ma non sono esposti nel
menu né inviati: il mapping CAN di questa demo espone solo RGB.

Se `vambienttemp` è attivo, i nuovi campioni temperatura possono impostare a
loro volta lo stesso segnale colore e sovrascrivere il valore manuale.

## Verifica minima

1. invia comando dal dashboard e verifica update locale
2. invia comando dal cloud app e verifica update su switch e dashboard
3. verifica telemetria cloud
