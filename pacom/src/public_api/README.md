# Modulo `public_api`

Questa cartella contiene il codice che rappresenta l'**interfaccia pubblica** della libreria PACOM. È il codice che l'utente finale del framework vedrà e chiamerà.

L'obiettivo di questo modulo è l'**ergonomia**. Poiché le API sottostanti di uProtocol e i concetti di RPC asincroni possono essere molto ostici, il `public_api` avvolge tutta la complessità dietro metodi dal nome esplicito e facile da capire.

## Struttura del codice

L'unico (o principale) file in questa cartella è `mod.rs`.

### `PacomRuntime`

La struttura principale esportata è (sebbene spesso definita materialmente nel `runtime` ed esportata qui, o wrappata). L'utente crea l'istanza con `PacomRuntime::new(...)` e riceve un oggetto thread-safe (spesso internamente incapsulato in un `Arc`) che può essere clonato e passato a diversi task asincroni.

### Metodi Principali e loro Funzionamento

Ecco una spiegazione di cosa fanno sotto il cofano le API pubbliche:

#### 1. Pattern Publish/Subscribe

- **`publish_event(topic: &str, payload: Vec<u8>)`**:
  Invia un evento senza aspettare risposta. Sotto il cofano, delega al runtime la validazione che `topic` esista nel manifest (nella sezione `topics.publish`), crea un `UUri` numerico e fa il broadcast via `UTransport`.
  
- **`subscribe_event(topic: &str, callback)`**:
  Registra una callback asincrona. Quando un messaggio arriva per quel `topic`, la callback viene eseguita con il payload in bytes. Sotto il cofano, implementa il trait `UListener` di uProtocol per agganciarsi al sistema di ricezione messaggi di vSomeIP/MQTT.

- **Varianti `_to` e `_from`** (`publish_event_to`, `subscribe_event_from`):
  Questi metodi accettano un argomento extra: l'`authority`. Di default PACOM usa l'autorità veicolare locale. Se l'utente vuole mandare un messaggio al cloud (tramite il bridge MQTT), deve specificare l'authority remota (es. `"cloud.bridge"`). Il router sottostante userà questa informazione per non mandare il pacchetto sul bus veicolare.

#### 2. Pattern RPC (Request / Response)

- **`register_rpc_method(method: &str, handler)`**:
  Serve per creare un server. L'utente passa il nome del metodo (es. `"/rpc/echo"`) e una closure asincrona che prende i bytes di richiesta e restituisce i bytes di risposta. Sotto il cofano, l'API wrappa l'handler utente in una struttura che implementa il `RequestHandler` trait di uProtocol, passandolo al `InMemoryRpcServer`.

- **`invoke_method(method: &str, payload: Vec<u8>)`**:
  Serve per chiamare un server (client RPC). La complessità nascosta qui è alta: 
  1. Si blocca in attesa che il meccanismo di discovery trovi l'IP/ID del server.
  2. Crea un pacchetto di request.
  3. Mette in pausa il thread chiamante in attesa della risposta (via `InMemoryRpcClient`).
  4. Risolve la promise restituendo la risposta.

## Come contribuire a `public_api`

- **Aggiunta di Metodi**: Se aggiungi un nuovo metodo, assicurati che la firma sia semplice e che tutti gli errori restituiti siano varianti di `PacomError`.
- **Nessuna logica complessa qui**: Questo file non dovrebbe contenere logiche di serializzazione, hash o networking. Se ti trovi a scrivere più di 20 righe per un metodo qui, probabilmente la logica appartiene a `runtime::engine.rs`. Il `public_api` deve essere solo un passacarte (facade pattern).
