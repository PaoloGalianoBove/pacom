# Deploy MQTT/SOME-IP con Docker (Virtual Network Mode)

Questa guida mostra come eseguire le applicazioni in Docker utilizzando la rete virtuale di Docker (`pacom-net`). Questo garantisce che i container abbiano indirizzi IP distinti, evitando conflitti di porta e consentendo a SOME/IP di funzionare correttamente a livello di rete.

---

## 1. Build dell'Immagine Docker
Dalla cartella principale del progetto (`/workspaces/pacom-develop/pacom`), compila l'immagine contenente gli eseguibili Rust pronti:

```bash
docker build -t pacom-demo:latest .
```

---

## 2. Esecuzione dei Componenti

### 2.0. Avvia Mosquitto (Broker MQTT)
```bash
docker run -d --name mosquitto --network pacom-net -p 1883:1883 eclipse-mosquitto:1.6
```

### 2.1. Avvia App 2: `light_switch` (ECU Switch / Controller Core)
Questo container simula la centralina principale del veicolo (ECU 1). Esegue direttamente il binario Rust `/opt/pacom/bin/light_switch`:
```bash
docker run -it --rm --name ecu-switch --network pacom-net \
  -v /tmp/pacom-ipc:/tmp \
  -e UP_AUTHORITY=ecu-switch \
  -e UP_UE_ID=0x3301 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/light-switch/manifest.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  -e PACOM_DEBUG_VERBOSE=true \
  -e PACOM_ENABLE_LOCAL_WILDCARD_SUBSCRIBE=false \
  pacom-demo:latest \
  /opt/pacom/bin/light_switch
```

### 2.2. Avvia App 1: `light_dashboard` (ECU Dashboard / HMI locale)
Questo container simula una seconda centralina nel veicolo (ECU 2). Esegue direttamente il binario `/opt/pacom/bin/light_dashboard`:
```bash
docker run -it --rm --name ecu-dashboard --network pacom-net \
  -v /tmp/pacom-ipc:/tmp \
  -e UP_AUTHORITY=ecu-dashboard \
  -e UP_UE_ID=0x1234 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/light-dashboard/manifest.json \
  -e PACOM_DEBUG_VERBOSE=true \
  -e PACOM_ENABLE_LOCAL_WILDCARD_SUBSCRIBE=false \
  pacom-demo:latest \
  /opt/pacom/bin/light_dashboard
```

### 2.3. Avvia App 3: `cloud_app` (Simulatore Cloud esterno)
Simula l'infrastruttura Cloud. Esegue il binario `/opt/pacom/bin/cloud_app`:
```bash
docker run -it --rm --name cloud-app --network pacom-net \
  -v /tmp/pacom-ipc:/tmp \
  -e UP_AUTHORITY=cloud.bridge \
  -e UP_UE_ID=0x2200 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/cloud-app/manifest.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  -e PACOM_DISABLE_VSOMEIP=true \
  -e PACOM_DEBUG_VERBOSE=true \
  pacom-demo:latest \
  /opt/pacom/bin/cloud_app
```

---

## 3. Simulazione e Logica di Concorrenza
*   Digita i comandi da `ecu-dashboard` (SOME/IP locale) ed osserva i cambi di stato.
*   Digita i comandi da `cloud-app` (MQTT) per simulare l'attivazione remota.
*   Invia un comando da `cloud-app` e subito dopo (entro 1.5 secondi) seleziona un'opzione diversa da `ecu-dashboard` per vedere lo scarto automatico del comando cloud in favore di quello locale.

## 4. Note di bootstrap

- L'ordine di startup `switch -> dashboard` e supportato: la discovery periodica riallinea la subscribe.
- Gli eventi topic sono live: se dashboard parte dopo il primo evento, vedra lo stato al prossimo evento.
- Il primo comando cloud puo essere perso se inviato prima che switch completi la subscribe su `/cloud/command`.
- Per test robusti, attendere alcuni secondi prima del primissimo comando cloud.
