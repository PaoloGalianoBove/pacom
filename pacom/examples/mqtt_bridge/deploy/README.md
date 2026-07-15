# Deploy MQTT Bridge con Docker

Questa guida mostra come buildare l'immagine Docker di PACOM e avviare i 4 componenti dell'esempio MQTT Bridge uno per volta (ideale per una demo live per spiegare pezzo per pezzo).

## 1. Build dell'Immagine Docker

Dalla cartella principale del progetto (`/workspaces/pacom-develop/pacom`), esegui la build dell'immagine. Il Dockerfile è stato configurato per compilare e includere tutti gli eseguibili dell'esempio.

```bash
docker build -t pacom-demo:latest .
```

## 2. Creazione della rete e del Volume IPC

Per far comunicare Hub ed Edge in locale tramite vSomeIP (senza passare dal networking IP), creiamo un volume Docker condiviso.
Creeremo anche una rete Docker dedicata per far sì che i container si vedano, ma ricordati che l'Hub e l'Edge comunicheranno **solo tramite il volume IPC** per la parte intra-veicolo.

```bash
# Crea un volume condiviso per i socket vSomeIP (/tmp)
docker volume create pacom-ipc

# Crea una rete per la comunicazione Docker (serve a Mosquitto)
docker network create pacom-net
```

## 3. Esecuzione passo-passo (La Demo)

> **Nota per Mosquitto:** Se hai già mosquitto in esecuzione sul tuo host Linux, puoi saltare il punto 3.0 e usare `--net host` o l'IP dell'host. Altrimenti, lancia Mosquitto dentro Docker come segue:

### 3.0. Avvia Mosquitto (Broker MQTT)
```bash
docker run -d --name mosquitto --network pacom-net -p 1883:1883 eclipse-mosquitto:1.6
```

### 3.1. Avvia l'Hub (Il Gateway Veicolo-Cloud)
Spiegazione: L'Hub si attacca sia al volume IPC (per parlare con l'Edge) sia alla rete MQTT.
```bash
docker run -it --rm --name pacom-hub \
  --network pacom-net \
  -v pacom-ipc:/tmp \
  -e APP_BIN=mqtt_bridge_hub \
  -e UP_AUTHORITY=ecu-hub \
  -e UP_UE_ID=0x3301 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/hub/manifest.json \
  -e PACOM_VSOMEIP_TEMPLATE_PATH=/opt/pacom/examples/rtt/deploy/vsomeip-router.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  -e PACOM_CLOUD_UE_ID=0x2200 \
  pacom-demo:latest
```

### 3.2. Avvia il Probe (Monitor Cloud)
Spiegazione: Il Probe simula il Cloud. Non ha bisogno del volume IPC (vSomeIP) perché parla solo MQTT!
```bash
docker run -it --rm --name pacom-probe \
  --network pacom-net \
  -e APP_BIN=mqtt_bridge_probe \
  -e UP_AUTHORITY=ecu-probe \
  -e UP_UE_ID=0x3303 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/probe/manifest.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  -e PACOM_CLOUD_UE_ID=0x2200 \
  pacom-demo:latest
```

### 3.3. Avvia l'Edge (Centralina in Auto)
Spiegazione: L'Edge si attacca SOLO al volume IPC. **Non ha accesso al broker MQTT**, è completamente isolato dal cloud. Comunicherà solo con l'Hub tramite vSomeIP!
```bash
docker run -it --rm --name pacom-edge \
  -v pacom-ipc:/tmp \
  -e APP_BIN=mqtt_bridge_edge \
  -e UP_AUTHORITY=ecu-edge \
  -e UP_UE_ID=0x3302 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/edge/manifest.json \
  pacom-demo:latest
```

### 3.4. Avvia il Sender (Comando dal Cloud)
Spiegazione: Il Sender spara un comando MQTT che l'Hub riceverà e inoltrerà all'Edge.
```bash
docker run -it --rm --name pacom-sender \
  --network pacom-net \
  -e APP_BIN=mqtt_bridge_sender \
  -e UP_AUTHORITY=ecu-sender \
  -e UP_UE_ID=0x3304 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/mqtt_bridge/sender/manifest.json \
  -e PACOM_MQTT_BROKER_URI=tcp://mosquitto:1883 \
  pacom-demo:latest
```

Ora puoi spiegare esattamente la topologia! L'Edge produce un dato isolato, l'Hub lo raccoglie da `/tmp/vsomeip-0` e lo sputa su TCP verso `mosquitto:1883`!
