# PACOM - Audit Definitivo Completo (src/)

Analisi statica completa dei sorgenti sotto `src/` (3.527 righe Rust), con consolidamento di:
- punti gia risolti,
- punti aperti originali,
- punti aggiuntivi emersi durante il controllo riga per riga.

Data audit: 2026-07-29
Ambito: `src/**/*.rs`

Stato implementazione rapida: aggiornato al 2026-07-29

## 0) Riepilogo implementazione rapida

| Categoria | Conteggio |
|---|---:|
| Fix rapidi implementati | 18 |
| Fix non rapidi (refactor/design) | 4 |
| N/A | 1 |
| Punti gia risolti | 3 |

Dettaglio non implementati perche non rapidi: `A1`, `A2`, `M7`, `M12`.

| Punto | Fix rapido? | Stato implementazione |
|---|---|---|
| A1 | No | Non implementato (refactoring) |
| A2 | No | Non implementato (refactoring) |
| A3 | Si | ✅ Implementato |
| A4 | Si | ✅ Implementato |
| A5 | Si | ✅ Implementato |
| A6 | Si | ✅ Implementato |
| M1 | Si | ✅ Implementato |
| M2 | Si | ✅ Implementato |
| M3 | Si | ✅ Implementato |
| M4 | Si | ✅ Implementato |
| M5 | Si | ✅ Implementato |
| M6 | Si | ✅ Implementato |
| M7 | No | Non implementato (design) |
| M8 | Si | ✅ Implementato |
| M9 | Si | ✅ Implementato |
| M10 | Si | ✅ Implementato |
| M11 | Si (doc) | ✅ Implementato |
| M12 | No | Non implementato (design) |
| M13 | Si | ✅ Implementato |
| M14 | Si | ✅ Implementato |
| B1 | N/A | N/A |
| B2 | Si | ✅ Implementato |
| B3 | Si | ✅ Implementato |

---

## 1) Punti gia risolti

| # | File | Descrizione | Stato | Fix rapido? |
|---|------|-------------|-------|-------------|
| R1 | `src/transport/mqtt.rs` | Hard-limit MQTT 100/10 alzato a 10.000/100 | ✅ Risolto | Gia fatto |
| R2 | `src/transport/vsomeip_topology.rs` | `dedup_by` non eliminava duplicati non adiacenti -> `HashSet` | ✅ Risolto | Gia fatto |
| R3 | `src/transport/vsomeip.rs` | Socket IPC fuori dalla cartella condivisa -> spostato in `IPC_DIR` | ✅ Risolto | Gia fatto |

---

## 2) Criticita alte

| # | File | Descrizione | Gravita | Fix rapido? | Stato attuale |
|---|------|-------------|---------|-------------|--------------|
| A1 | `src/transport/router.rs`, `src/runtime/engine.rs` | Nessuna astrazione `Transport` (dipendenze concrete vSomeIP/MQTT) | 🟠 Alta | No | Non implementato |
| A2 | `src/runtime/engine.rs` | `engine.rs` God Object (1262 righe) con discovery accoppiata al core runtime | 🟠 Alta | No | Non implementato |
| A3 | `src/transport/router.rs` | `receive()` delega solo a vSomeIP; nodo MQTT-only riceve `UNAVAILABLE` | 🟠 Alta | Si | ✅ Implementato |
| A4 | `src/runtime/engine.rs` | In `publish()`, discovery announce avviene dopo `send()` (inconsistenza parziale) | 🟠 Alta | Si | ✅ Implementato |
| A5 (nuovo) | `src/transport/router.rs` | Cloud-bound send con MQTT non configurato: log + `Ok(())` (drop silenzioso) | 🟠 Alta | Si | ✅ Implementato |
| A6 (nuovo) | `src/runtime/engine.rs` | `subscribe_from_authority()` perde il vincolo authority quando va in pending | 🟠 Alta | Si | ✅ Implementato |

---

## 3) Criticita medie

| # | File | Descrizione | Gravita | Fix rapido? | Stato attuale |
|---|------|-------------|---------|-------------|--------------|
| M1 | `src/runtime/engine.rs` | `resolve_ue_id()` richiamato ad ogni operazione (non cachato in `new()`) | 🟡 Media | Si | ✅ Implementato |
| M2 | `src/runtime/engine.rs`, `src/transport/vsomeip.rs` | 16 canali discovery hardcoded, non configurabili | 🟡 Media | Si | ✅ Implementato |
| M3 | `src/transport/vsomeip.rs` | File config temporanei in `/tmp` non ripuliti | 🟡 Media | Si | ✅ Implementato |
| M4 | `src/runtime/engine.rs` | Timeout RPC hardcoded a 5000ms | 🟡 Media | Si | ✅ Implementato |
| M5 | `src/transport/vsomeip.rs` | `APP_NAME` e `APP_ID_HEX` fuori convenzione `PACOM_*` | 🟡 Media | Si | ✅ Implementato |
| M6 | `src/runtime/engine.rs` | Fallback authority `local_ecu` ambiguo in multi-ECU | 🟡 Media | Si | ✅ Implementato |
| M7 | `src/runtime/engine.rs` | `major_version` hardcoded a `1` ovunque | 🟡 Media | No | Non implementato |
| M8 | `src/runtime/engine.rs` | `DiscoveryEvent` senza `protocol_version` | 🟡 Media | Si | ✅ Implementato |
| M9 | `src/runtime/engine.rs`, `src/public_api/mod.rs` | Nessun graceful shutdown (`reannounce` task senza cancellation) | 🟡 Media | Si | ✅ Implementato |
| M10 | `src/transport/vsomeip.rs` | `get_local_ip()` fallback a `127.0.0.1` in ambienti air-gapped | 🟡 Media | Si | ✅ Implementato |
| M11 | `src/transport/vsomeip.rs` | `lock_owner_is_dead()` dipende da `/proc` (Linux-only) | 🟡 Media | Si (doc) | ✅ Implementato |
| M12 (nuovo) | `src/runtime/engine.rs` | Discovery cache 1:1 per nome (ultimo provider sovrascrive i precedenti) | 🟡 Media | No | Non implementato |
| M13 (nuovo) | `src/transport/router.rs` | Incoerenza tra docstring di `is_cloud_bound()` e logica effettiva | 🟡 Media | Si | ✅ Implementato |
| M14 (nuovo) | `src/runtime/logical_registry.rs` | `ManifestConfig::load()` fallisce in modo silenzioso (default vuoto) | 🟡 Media | Si | ✅ Implementato |

---

## 4) Criticita basse

| # | File | Descrizione | Gravita | Fix rapido? | Stato attuale |
|---|------|-------------|---------|-------------|--------------|
| B1 | `src/transport/vsomeip.rs` | `unsafe { set_var(...) }` a runtime (trade-off accettato) | ⚪ Bassa | N/A | N/A |
| B2 | `src/runtime/engine.rs` | Errore RPC mappato su `PacomError::Config(...)` (semantica discutibile) | ⚪ Bassa | Si | ✅ Implementato |
| B3 | `src/public_api/mod.rs` | Distinzione `publish()` vs `publish_event_to()` non abbastanza esplicita in docs | ⚪ Bassa | Si | ✅ Implementato |

---

## 5) Totale complessivo aggiornato

| Categoria | Conteggio |
|---|---:|
| Risolte | 3 |
| Alte totali | 6 |
| Medie totali | 14 |
| Basse totali | 3 |
| Implementate in questa passata | 18 |
| Non implementate (non rapide) | 4 |
| **Totale punti** | **26** |

Formula: `3 + 6 + 14 + 3 = 26`

---

## 6) Priorita consigliata di intervento

1. A1 - introdurre astrazione transport (refactor architetturale).
2. A2 - separare discovery dal God Object `engine.rs`.
3. M12 - supportare multi-provider per stesso logical name.
4. M7 - versioning del servizio non hardcoded a `major_version = 1`.

---

## 7) Note di metodo

- Audit statico dei sorgenti Rust in `src/`.
- Non include validazione dinamica con test end-to-end o fault injection.
- Le priorita riflettono impatto operativo e rischio di comportamento inatteso.
