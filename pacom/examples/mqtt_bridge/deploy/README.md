# Deploy MQTT/SOME-IP con Docker (Virtual Network Mode)

Questa guida mostra come eseguire le applicazioni in Docker utilizzando la rete virtuale di Docker (`pacom-net`).
I container hanno indirizzi IP distinti, evitando conflitti di porta e consentendo a SOME/IP di funzionare correttamente a livello di rete.

---

## 0. Prerequisiti

### Crea la rete Docker
```bash
docker network create pacom-net
```

### Build dell'immagine
Dalla cartella radice del progetto (dove si trova il `Dockerfile`):
```bash
docker build -t pacom-demo:latest -f Dockerfile .
```

> **Nota:** il Dockerfile usa uno stage `builder` che compila tutti gli esempi in release.
> Assicurati che il `Dockerfile` copi i binari `light_switch`, `light_dashboard` e `cloud_app`
> in `/opt/pacom/bin/` oltre ai binari RTT.

---

## 1. Avvia il Broker MQTT (Mosquitto)

```bash
docker run -d \
  --name mosquitto \
  --network pacom-net \
  -p 1883:1883 \
  eclipse-mosquitto:1.6
```

---

## 2. Avvia `light_switch` (ECU Switch — Controller principale)

Simula la centralina principale del veicolo. Gestisce RPC locali via SOME/IP e comunica col cloud via MQTT.

```bash
docker run -it --rm \
  --name ecu-switch \
  --network pacom-net \
  -e UP_AUTHORITY=ecu-switch \
  -e UP_UE_ID=0x3301 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/light-switch/manifest.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  -e PACOM_CLOUD_AUTHORITY=cloud.bridge \
  pacom-demo:latest \
  /opt/pacom/bin/light_switch
```

---

## 3. Avvia `light_dashboard` (ECU Dashboard — HMI locale)

Simula una seconda centralina nel veicolo. Usa **solo SOME/IP**, senza MQTT.

```bash
docker run -it --rm \
  --name ecu-dashboard \
  --network pacom-net \
  -e UP_AUTHORITY=ecu-dashboard \
  -e UP_UE_ID=0x1234 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/light-dashboard/manifest.json \
  pacom-demo:latest \
  /opt/pacom/bin/light_dashboard
```

---

## 4. Avvia `cloud_app` (Simulatore Cloud esterno)

Simula l'infrastruttura Cloud. Usa **solo MQTT**, senza SOME/IP (vSomeIP disabilitato).

```bash
docker run -it --rm \
  --name cloud-app \
  --network pacom-net \
  -e UP_AUTHORITY=cloud.bridge \
  -e UP_UE_ID=0x2200 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/cloud-app/manifest.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  -e PACOM_CLOUD_AUTHORITY=cloud.bridge \
  -e PACOM_DISABLE_VSOMEIP=true \
  pacom-demo:latest \
  /opt/pacom/bin/cloud_app
```

---

## 5. Ordine di Avvio Consigliato

```
1. mosquitto   (broker MQTT — deve essere pronto prima di qualsiasi app con MQTT)
2. light_switch  (publisher SOME/IP + subscriber MQTT cloud)
3. light_dashboard  (subscriber SOME/IP — si connette via discovery a switch)
4. cloud_app  (subscriber e publisher MQTT cloud)
```

> `light_dashboard` può partire prima o dopo `light_switch` grazie al meccanismo
> di pending subscriptions e periodic discovery di pacom.

---

## 6. Variabili d'Ambiente di Riferimento

| Variabile | Descrizione | Default |
|---|---|---|
| `UP_AUTHORITY` | Nome logico del nodo (ECU) | `HOSTNAME` del container |
| `UP_UE_ID` | ID univoco dell'applicazione (hex) | Hash del nome eseguibile |
| `PACOM_MANIFEST_PATH` | Path del manifest JSON | `/etc/pacom/manifest.json` |
| `PACOM_MQTT_BROKER_URI` | URI del broker MQTT | `mqtt://127.0.0.1:1883` |
| `PACOM_CLOUD_AUTHORITY` | **Obbligatoria** — Authority della sink cloud | *(nessun default)* |
| `PACOM_DISABLE_VSOMEIP` | Disabilita il transport vSomeIP | `false` |
| `PACOM_VSOMEIP_ROLE` | Forza il ruolo vSomeIP (`router`/`client`) | Auto-election |
| `PACOM_DEBUG_VERBOSE` | Log verbosi di pacom | `false` |

---

## 7. Prova Rapida

Una volta avviati tutti i container:

- Da `light_dashboard`: scegli `1` (Low Beam) → comando RPC via SOME/IP → `light_switch` risponde.
- Da `cloud_app`: scegli `2` (High Beam) → comando MQTT → `light_switch` riceve e aggiorna lo stato.
- Osserva che `light_dashboard` aggiorna il suo stato anche dopo un comando cloud (via SOME/IP topic `/status/lights`).

---

## 8. Note Tecniche

- I due container ECU (`ecu-switch` e `ecu-dashboard`) comunicano tra loro **via SOME/IP UDP** sulla rete `pacom-net`.
- `cloud_app` non partecipa mai al bus SOME/IP — interagisce solo tramite il broker Mosquitto.
- `PACOM_CLOUD_AUTHORITY` deve essere identica su tutti i container che producono o consumano topic `/cloud/*`. In questo setup è sempre `cloud.bridge`.


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
